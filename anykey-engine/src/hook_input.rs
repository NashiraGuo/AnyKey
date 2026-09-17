//! hook_input.rs — 免驱动输入后端：`WH_KEYBOARD_LL` + 专用线程消息泵。
//!
//! 这个模块是"便携模式"的输入侧。它产出的 `AnyKeyInputEvent` 与驱动后端逐字段同构
//! （`make_code` = 裸扫描码、`flags` = BREAK/E0/E1），所以整条 pipeline 不需要任何改动。
//!
//! # 三条硬约束（都来自实测，见 docs/design_portable_nodriver_backend.md 与 tools/probe_*）
//!
//! 1. **回调在安装钩子的那条线程上下发，那条线程必须有消息循环。**
//!    所以钩子装在专用线程上，跑 `MsgWaitForMultipleObjectsEx` 泵（与 `app_sensor.rs` 同一模式）。
//!    线程退出 = 钩子失效，因此泵循环只在收到 WM_QUIT 时返回。
//!
//! 2. **回调绝不阻塞。** 判定 → `try_send` 进有界通道 → 立即返回。
//!    通道满（管道失速）时**放行**该键（直通降级）：宁可这一次不映射，也绝不丢键。
//!    ⚠️ 不要学 Kanata 的 `try_send_panic`（回调里 panic = 键盘卡死）。
//!
//! 3. **本进程绝不注册键盘 Raw Input。** 实测：同进程一旦 `RegisterRawInputDevices`
//!    注册键盘 TLC，LL hook 立即停止被调用（反注册后恢复）。这也是便携模式暂不接管
//!    鼠标的原因之一（要接管鼠标得单独起进程或改用别的手段，见设计文档）。
//!
//! # 自注入识别
//!
//! 只用 `LLKHF_INJECTED` 这一位，且**必须放行**：本后端的输出走 `SendInput`，那会再次
//! 经过本钩子；若把它也吞掉，就会出现"自己的输出被自己吞掉 → 应用永远收不到键"的死锁。
//! 不用 `dwExtraInfo` 打标记（Kanata 也不打），少一层约定。

use crate::filter_driver::{
    AnyKeyInputEvent, AnyKeyMouseEvent, AnyKeyOutputEvent, ANYKEY_KEY_BREAK, ANYKEY_KEY_E0,
    ANYKEY_KEY_E1,
};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::OnceLock;
use std::time::Duration;
use windows_sys::Win32::Foundation::{GetLastError, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLKHF_UP,
    MsgWaitForMultipleObjectsEx, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, MSG, PM_REMOVE, QS_ALLINPUT, WH_KEYBOARD_LL, WM_QUIT,
};

/// 有界通道容量。管道正常时空闲；只有在管道失速（例如某个 Run: 卡住）时才会满，
/// 满了就直通降级，不丢键。
const CHANNEL_CAP: usize = 512;
/// 单次 `poll_all` 最多带走的条数（防止一次处理过多导致下一轮定时器饥饿）。
const BATCH_MAX: usize = 256;
/// 泵线程的唤醒间隔（毫秒）。只用于让线程可被 WM_QUIT 唤醒，不承载逻辑。
const PUMP_TICK_MS: u32 = 1000;
/// 安装等待上限：泵线程装钩子失败/卡住时不要把进程挂死。
const INSTALL_TIMEOUT_MS: u64 = 5000;
/// 判定"钩子疑似失活"的门槛：系统侧有输入、而本钩子连续这么久没有回调。
const HOOK_SILENT_LIMIT_MS: u32 = 5000;

// ── 全模块共享状态 ──
//
// 钩子回调是 `extern "system" fn`，没有 user data 参数，只能经静态取得上下文。
// `Shared` 里全是原子量：回调路径上不做任何加锁（`SyncSender::try_send` 自身即可重入安全）。
struct Shared {
    tx: SyncSender<AnyKeyInputEvent>,
    /// 是否吞键。A2 决策 = 装上即吞所有物理键；置 false 时为纯透传（撤下闸门）。
    swallow: AtomicBool,
    /// 钩子回调次数（成功入队才算）。main loop 靠它判断"装上≠收得到"。
    callbacks: AtomicU64,
    /// 最后一次成功入队的系统 tick（`GetTickCount` 基准）。
    last_cb_tick: AtomicU32,
    /// 安装时刻的系统 tick —— 还没收到任何回调时用它作对账基准。
    install_tick: AtomicU32,
    /// 因通道满/断连而放行（直通降级）的按键数。
    passthrough: AtomicU64,
    /// 泵线程 id（Drop 时 PostThreadMessageW(WM_QUIT) 用）。
    pump_tid: AtomicU32,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

// ── 事件投影（纯函数，可单测） ──

/// 把 `KBDLLHOOKSTRUCT` 的字段投影成 AnyKey 的 `(make_code, flags)` 约定。
///
/// - `make_code` = **裸扫描码**（不含 0xE0 前缀；E0 用 flag 表达，与驱动一致）
/// - `flags`：`ANYKEY_KEY_BREAK`(0x01) 抬起 / `ANYKEY_KEY_E0`(0x02) 扩展 / `ANYKEY_KEY_E1`(0x04)
///
/// # E1 缺口与它的补法
///
/// `KBDLLHOOKSTRUCT` **没有 E1 位**（只有 `LLKHF_EXTENDED` 对应 E0），而驱动的 flags 里有独立的
/// `ANYKEY_KEY_E1`。丢了这个位，Pause 会和 NumLock 撞在一起 —— 两者扫描码都是 0x45，
/// `input_name(0x45, E1)` = "pause"、`input_name(0x45, 0)` = "numlock"。
/// 好消息是两者**虚拟键码不同**（Pause = VK_PAUSE 0x13 / NumLock = VK_NUMLOCK 0x90），
/// 所以用 `vk_code` 兜住这一个特例即可。这是本后端唯一一处"投影不完整"的地方。
pub fn project(vk_code: u32, scan_code: u32, hook_flags: u32) -> (u16, u16) {
    let mut flags: u16 = 0;
    if hook_flags & LLKHF_UP != 0 {
        flags |= ANYKEY_KEY_BREAK;
    }
    if hook_flags & LLKHF_EXTENDED != 0 {
        flags |= ANYKEY_KEY_E0;
    }
    // Pause（E1 0x45）在 hook 里只剩扫描码，用 vkCode 与 NumLock 区分。
    if scan_code == 0x45 && (vk_code & 0xFF) == 0x13 {
        flags |= ANYKEY_KEY_E1;
    }
    (scan_code as u16, flags)
}

// ── 钩子回调 ──

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // 整个函数体放在显式 unsafe 块里：内部全是裸指针操作（解引用 lParam、调用 Win32），
    // 而且闭包不继承 `unsafe fn` 的 unsafe 上下文，显式写出更不容易出错。
    unsafe {
        let next = |code: i32, wparam: WPARAM, lparam: LPARAM| -> LRESULT {
            CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
        };

        // nCode < 0：按文档必须原样交给下一个钩子，不做任何处理。
        if code < 0 {
            return next(code, wparam, lparam);
        }
        let Some(sh) = SHARED.get() else {
            return next(code, wparam, lparam);
        };
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
        let flags = kb.flags;

        // 自注入放行：本后端输出走 SendInput，会再次回到这里。吞掉它 = 自己的输出被自己吞掉。
        if flags & LLKHF_INJECTED != 0 {
            return next(code, wparam, lparam);
        }
        // 未开启吞键（尚未 armed / 已降级 / 正在退出）→ 纯透传。
        if !sh.swallow.load(Ordering::Relaxed) {
            return next(code, wparam, lparam);
        }

        let (make_code, ak_flags) = project(kb.vkCode, kb.scanCode, flags);
        let ev = AnyKeyInputEvent {
            make_code,
            flags: ak_flags,
            // 便携模式无设备维度：全部归到伪设备 0（见 backend.rs 的单设备桩）。
            device_id: 0,
            extra_info: kb.dwExtraInfo as u32,
        };

        match sh.tx.try_send(ev) {
            Ok(()) => {
                sh.callbacks.fetch_add(1, Ordering::Relaxed);
                sh.last_cb_tick.store(GetTickCount(), Ordering::Relaxed);
                1 // 吞掉：交给管道决定输出什么（A2 决策）
            }
            Err(_) => {
                // 通道满或泵已死 —— 直通降级。绝不在这里等待、绝不 panic。
                sh.passthrough.fetch_add(1, Ordering::Relaxed);
                next(code, wparam, lparam)
            }
        }
    }
}

// ── 泵线程 ──

fn pump_thread(install_tx: Sender<Result<(), String>>) {
    unsafe {
        // 先强制创建本线程的消息队列：PostThreadMessageW 与钩子回调都依赖它存在。
        let mut msg: MSG = std::mem::zeroed();
        PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, 0);
        if let Some(sh) = SHARED.get() {
            sh.pump_tid.store(GetCurrentThreadId(), Ordering::SeqCst);
            sh.install_tick.store(GetTickCount(), Ordering::SeqCst);
        }
        // hMod = NULL 对 WH_KEYBOARD_LL 是合法的（回调在当前进程内）。
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), std::ptr::null_mut(), 0);
        if hook.is_null() {
            let err = GetLastError();
            let _ = install_tx.send(Err(format!(
                "SetWindowsHookExW(WH_KEYBOARD_LL) failed, GetLastError={}",
                err
            )));
            return;
        }
        HOOK_HANDLE.store(hook, Ordering::SeqCst);
        let _ = install_tx.send(Ok(()));
    }

    // 消息泵：这是钩子能被回调的前提。
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let res = unsafe {
            MsgWaitForMultipleObjectsEx(0, std::ptr::null(), PUMP_TICK_MS, QS_ALLINPUT, 0)
        };
        if res == 0 {
            // WAIT_OBJECT_0：有消息（含钩子回调的派发）
            unsafe {
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    if msg.message == WM_QUIT {
                        return;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }
}

// ── 句柄存放（`HHOOK = *mut c_void` 不是 Send，用 AtomicPtr 跨线程传） ──

static HOOK_HANDLE: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

// ── 对外类型 ──

/// 钩子存活对账结果（供 main loop 打日志）。
pub struct HookLiveness {
    pub callbacks: u64,
    pub since_last_cb_ms: u32,
    pub system_idle_ms: u32,
    pub passthrough: u64,
    /// false = "系统侧有输入，而本钩子长时间没有回调"——可能是钩子被摘除，
    /// 也可能是前台在提权窗口（不提权运行时收不到，属预期）。
    pub healthy: bool,
}

pub struct HookInput {
    rx: Receiver<AnyKeyInputEvent>,
    hook: *mut std::ffi::c_void,
}

unsafe impl Send for HookInput {}

impl HookInput {
    /// 安装全局键盘低级钩子。成功后从那一刻起**所有物理键都会被吞掉**（A2 决策），
    /// 由管道逐个重放。
    pub fn install() -> Result<Self, String> {
        let (tx, rx) = sync_channel::<AnyKeyInputEvent>(CHANNEL_CAP);
        let (install_tx, install_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        SHARED
            .set(Shared {
                tx,
                swallow: AtomicBool::new(true),
                callbacks: AtomicU64::new(0),
                last_cb_tick: AtomicU32::new(0),
                install_tick: AtomicU32::new(0),
                passthrough: AtomicU64::new(0),
                pump_tid: AtomicU32::new(0),
            })
            .map_err(|_| "llhook backend already installed in this process".to_string())?;

        std::thread::Builder::new()
            .name("anykey-llhook-pump".into())
            .spawn(move || pump_thread(install_tx))
            .map_err(|e| format!("spawn pump thread failed: {}", e))?;

        match install_rx.recv_timeout(Duration::from_millis(INSTALL_TIMEOUT_MS)) {
            Ok(Ok(())) => {
                let hook = HOOK_HANDLE.load(Ordering::SeqCst);
                Ok(HookInput { rx, hook })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(format!(
                "hook install timed out ({}ms) — pump thread not responding",
                INSTALL_TIMEOUT_MS
            )),
        }
    }

    /// 等待输入。**超时返回空 Vec**（与驱动后端同语义）——主循环靠"返回空"这一刻推进
    /// combo / tap-dance / defer 的定时器。
    ///
    /// 便携模式不接管鼠标：mouse 侧恒为空（见模块头注释 3）。
    pub fn poll_all(
        &self,
        timeout_ms: u32,
    ) -> Result<(Vec<AnyKeyInputEvent>, Vec<AnyKeyMouseEvent>), String> {
        let mut kb: Vec<AnyKeyInputEvent> = Vec::with_capacity(16);
        match self.rx.recv_timeout(Duration::from_millis(timeout_ms as u64)) {
            Ok(e) => kb.push(e),
            Err(RecvTimeoutError::Timeout) => return Ok((kb, Vec::new())),
            Err(RecvTimeoutError::Disconnected) => {
                // 泵线程死了 → 钩子也已失效（键会原样透传），引擎应退出而不是继续跑。
                return Err("llhook pump thread exited (channel closed)".into());
            }
        }
        while kb.len() < BATCH_MAX {
            match self.rx.try_recv() {
                Ok(e) => kb.push(e),
                Err(_) => break,
            }
        }
        Ok((kb, Vec::new()))
    }

    /// 注入一个键盘输出事件（扫描码模式，复用 `emit.rs` 的 SendInput 实现）。
    /// 返回 Err 表示 `SendInput` 返回 0（UIPI / 安全软件拦截）——调用方应告警。
    pub fn send_output(&self, ev: &AnyKeyOutputEvent) -> Result<(), String> {
        let ext = (ev.flags & ANYKEY_KEY_E0) != 0;
        let is_up = (ev.flags & ANYKEY_KEY_BREAK) != 0;
        let sent = if is_up {
            crate::emit::send_key_up_sendinput(ev.make_code, ext)
        } else {
            crate::emit::send_key_down_sendinput(ev.make_code, ext)
        };
        if sent == 0 {
            return Err(format!(
                "SendInput returned 0 (sc=0x{:02X} up={} ext={}) — UIPI or security software",
                ev.make_code, is_up, ext
            ));
        }
        Ok(())
    }

    /// 钩子存活对账。判定式：**系统侧最近一次输入发生在"本钩子最后一次回调"之后，
    /// 且已持续超过 `HOOK_SILENT_LIMIT_MS`** → 有输入没经过我们。
    ///
    /// 用 `GetLastInputInfo()` 而不是自己的计数器，是因为它是系统级、**不经过本钩子**
    /// 的独立见证（实测验证过，见 tools/probe_elev_hook2.py）。
    pub fn liveness(&self) -> HookLiveness {
        let now = unsafe { GetTickCount() };
        let (callbacks, passthrough, baseline) = match SHARED.get() {
            Some(sh) => {
                let last = sh.last_cb_tick.load(Ordering::Relaxed);
                let base = if last != 0 {
                    last
                } else {
                    sh.install_tick.load(Ordering::Relaxed)
                };
                (
                    sh.callbacks.load(Ordering::Relaxed),
                    sh.passthrough.load(Ordering::Relaxed),
                    base,
                )
            }
            None => (0, 0, now),
        };
        let since_last_cb_ms = now.wrapping_sub(baseline);
        let system_idle_ms = system_idle_ms(now);
        // 系统侧输入的"年龄"比我们的静默时长更小 → 那次输入没经过本钩子。
        let missed = system_idle_ms < since_last_cb_ms;
        HookLiveness {
            callbacks,
            since_last_cb_ms,
            system_idle_ms,
            passthrough,
            healthy: !(missed && since_last_cb_ms > HOOK_SILENT_LIMIT_MS),
        }
    }
}

impl Drop for HookInput {
    fn drop(&mut self) {
        // 顺序：先撤闸门（停止吞键，在途回调立即变透传）→ 卸钩子 → 让泵线程退出。
        if let Some(sh) = SHARED.get() {
            sh.swallow.store(false, Ordering::SeqCst);
        }
        if !self.hook.is_null() {
            unsafe { UnhookWindowsHookEx(self.hook) };
            HOOK_HANDLE.store(std::ptr::null_mut(), Ordering::SeqCst);
        }
        if let Some(sh) = SHARED.get() {
            let tid = sh.pump_tid.load(Ordering::SeqCst);
            if tid != 0 {
                unsafe { PostThreadMessageW(tid, WM_QUIT, 0, 0) };
            }
        }
    }
}

/// `GetTickCount() - GetLastInputInfo().dwTime`，即"距上一次系统级输入过了多久"。
fn system_idle_ms(now: u32) -> u32 {
    let mut lii = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if unsafe { GetLastInputInfo(&mut lii) } == 0 {
        return u32::MAX; // 取不到 → 视为"系统侧无输入"，避免误判钩子失活
    }
    now.wrapping_sub(lii.dwTime)
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_plain_key_down() {
        // 'A' 按下：无修饰位
        assert_eq!(project(0x41, 0x1E, 0), (0x1E, 0));
    }

    #[test]
    fn project_plain_key_up() {
        assert_eq!(project(0x41, 0x1E, LLKHF_UP), (0x1E, ANYKEY_KEY_BREAK));
    }

    #[test]
    fn project_extended_key() {
        // 右 Ctrl：E0 1D → 裸扫描码 0x1D + E0 标志
        assert_eq!(project(0xA3, 0x1D, LLKHF_EXTENDED), (0x1D, ANYKEY_KEY_E0));
        // 右 Ctrl 抬起
        assert_eq!(
            project(0xA3, 0x1D, LLKHF_EXTENDED | LLKHF_UP),
            (0x1D, ANYKEY_KEY_E0 | ANYKEY_KEY_BREAK)
        );
    }

    #[test]
    fn project_printscreen_is_e0() {
        // PrintScreen：hook 报 scan 0x37 + E0 → 与驱动一致（input_name → "printscreen"）
        assert_eq!(project(0x2C, 0x37, LLKHF_EXTENDED), (0x37, ANYKEY_KEY_E0));
    }

    #[test]
    fn project_pause_gets_e1_via_vkcode() {
        // Pause 与 NumLock 扫描码同为 0x45，hook 都没有 E0 位；靠 vkCode 区分。
        assert_eq!(project(0x13, 0x45, 0), (0x45, ANYKEY_KEY_E1)); // VK_PAUSE
        assert_eq!(project(0x90, 0x45, 0), (0x45, 0)); // VK_NUMLOCK
    }

    #[test]
    fn project_never_sets_break_on_non_up() {
        for f in [0u32, LLKHF_EXTENDED, LLKHF_INJECTED] {
            assert_eq!(project(0x1E, 0x1E, f).1 & ANYKEY_KEY_BREAK, 0);
        }
    }
}
