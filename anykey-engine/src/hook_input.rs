//! hook_input.rs — 免驱动输入后端：`WH_KEYBOARD_LL` + `WH_MOUSE_LL` + 专用线程消息泵。
//!
//! 这个模块是"便携模式"的输入侧。它产出的事件与驱动后端**逐字段同构**
//! （键盘 `make_code` = 裸扫描码、`flags` = BREAK/E0/E1；鼠标 `button_flags` = ntddmou 位，
//! `button_data` = 滚轮增量），所以整条 pipeline 不需要任何改动。
//!
//! # 四条硬约束（都来自实测，见 docs/design_portable_nodriver_backend.md 与 tools/probe_*）
//!
//! 1. **回调在安装钩子的那条线程上下发，那条线程必须有消息循环。**
//!    所以两个钩子都装在同一条专用线程上，跑 `MsgWaitForMultipleObjectsEx` 泵
//!    （与 `app_sensor.rs` 同一模式）。线程退出 = 钩子失效，因此泵循环只在 WM_QUIT 时返回。
//!
//! 2. **回调绝不阻塞。** 判定 → `try_send` 进有界通道 → 立即返回。
//!    通道满（管道失速）时**放行**该事件（直通降级）：宁可这一次不映射，也绝不丢键。
//!    ⚠️ 不要学 Kanata 的 `try_send_panic`（回调里 panic = 键盘/鼠标卡死）。
//!
//! 3. **本进程绝不注册键盘 Raw Input。** 实测：同进程一旦 `RegisterRawInputDevices`
//!    注册键盘 TLC，LL hook 立即停止被调用（反注册后恢复）。
//!    便携模式**也不需要** Raw Input —— 它没有设备维度，设备身份没有用处。
//!
//! 4. **鼠标钩子与键盘钩子同线程共存已实测**（`tools/probe_llmouse.py`）：
//!    装上鼠标钩子后键盘钩子仍正常回调，放行的移动与点击都正常到达目标。
//!
//! # 鼠标事件模型（与驱动严格对齐）
//!
//! 驱动侧的规则是：**纯移动（`ButtonFlags == 0`）直接转发给系统、不入队**；
//! 只有**按键 / 滚轮**才进管道（`anykey_flt.c` 注释原文 "Pure movement …
//! forward to system immediately, no queue"）。本模块照此实现：
//!
//! | 钩子消息 | 处理 |
//! |---|---|
//! | `WM_MOUSEMOVE` | **放行**（只计数，不产生事件）—— 高频事件走"吞+重放"是性能灾难 |
//! | 按键 / 滚轮 | 造 `AnyKeyMouseEvent` 入队并**吞掉**，由管道决定输出什么 |
//!
//! `flags` 恒为 `MOUSE_MOVE_RELATIVE`、`last_x/last_y` 恒为 0：这两个字段描述的是
//! **同一数据包里的位移**，而按键/滚轮包里没有位移语义；驱动那边它们也只是随包携带、
//! 引擎侧仅用于日志，不参与任何判定。
//!
//! # 自注入识别
//!
//! 只用 `LLKHF_INJECTED` / `LLMHF_INJECTED` 这一位，且**必须放行**：本后端的输出走
//! `SendInput`，那会再次经过本钩子；若把它也吞掉，就会出现"自己的输出被自己吞掉 →
//! 应用永远收不到"的死锁。不用 `dwExtraInfo` 打标记（Kanata 也不打），少一层约定。

use crate::events::{
    AnyKeyInputEvent, AnyKeyMouseEvent, AnyKeyOutputEvent, ANYKEY_KEY_BREAK, ANYKEY_KEY_E0,
    ANYKEY_KEY_E1, MOUSE_BUTTON_4_DOWN, MOUSE_BUTTON_4_UP, MOUSE_BUTTON_5_DOWN,
    MOUSE_BUTTON_5_UP, MOUSE_HWHEEL, MOUSE_LEFT_BUTTON_DOWN, MOUSE_LEFT_BUTTON_UP,
    MOUSE_MIDDLE_BUTTON_DOWN, MOUSE_MIDDLE_BUTTON_UP, MOUSE_MOVE_RELATIVE, MOUSE_RIGHT_BUTTON_DOWN,
    MOUSE_RIGHT_BUTTON_UP, MOUSE_WHEEL,
};
// XBUTTON1/2 的系统定义只在 ntddmou / winuser 头里，windows-sys 未导出；
// 单一来源放在 sendinput_out.rs（输出侧也要用），这里引用同一份。
use crate::sendinput_out::{XBUTTON1, XBUTTON2};
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
    LLMHF_INJECTED, MSLLHOOKSTRUCT, MsgWaitForMultipleObjectsEx, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, MSG, PM_REMOVE, QS_ALLINPUT,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_XBUTTONDOWN, WM_XBUTTONUP,
};

/// 有界通道容量。管道正常时空闲；只有在管道失速（例如某个 Run: 卡住）时才会满，
/// 满了就直通降级，不丢事件。
const CHANNEL_CAP: usize = 512;
/// 单次 `poll_all` 最多带走的条数（键盘 + 鼠标合计），防止一次处理过多导致下一轮定时器饥饿。
const BATCH_MAX: usize = 256;
/// 泵线程的唤醒间隔（毫秒）。只用于让线程可被 WM_QUIT 唤醒，不承载逻辑。
const PUMP_TICK_MS: u32 = 1000;
/// 安装等待上限：泵线程装钩子失败/卡住时不要把进程挂死。
const INSTALL_TIMEOUT_MS: u64 = 5000;
/// 判定"钩子疑似失活"的门槛：系统侧有输入、而本钩子连续这么久没有回调。
const HOOK_SILENT_LIMIT_MS: u32 = 5000;

// ── 通道载荷 ──
//
// 键盘与鼠标走**同一条通道**：主循环是单线程顺序消费，两条通道会让"等键盘"与"等鼠标"
// 互相阻塞（`recv_timeout` 只能等一条）。这与驱动后端"两个队列、同一次 poll 取走"的语义一致。
enum HookEvent {
    Key(AnyKeyInputEvent),
    Mouse(AnyKeyMouseEvent),
}

// ── 全模块共享状态 ──
//
// 钩子回调是 `extern "system" fn`，没有 user data 参数，只能经静态取得上下文。
// `Shared` 里全是原子量：回调路径上不做任何加锁（`SyncSender::try_send` 自身即可重入安全）。
struct Shared {
    tx: SyncSender<HookEvent>,
    /// 是否吞键/吞鼠标按键。A2 决策 = 装上即吞所有物理键；置 false 时为纯透传（撤下闸门）。
    swallow: AtomicBool,
    /// 键盘回调次数（成功入队才算）。main loop 靠它判断"装上≠收得到"。
    callbacks: AtomicU64,
    /// 最后一次**任意**钩子回调（键盘或鼠标，含纯移动与放行的注入事件）的系统 tick。
    /// ⚠️ 必须是"被调用的时刻"，不是"入队成功的时刻"——否则只动鼠标不打字时会误报失活。
    last_cb_tick: AtomicU32,
    /// 安装时刻的系统 tick —— 还没收到任何回调时用它作对账基准。
    install_tick: AtomicU32,
    /// 因通道满/断连而放行（直通降级）的事件数。
    passthrough: AtomicU64,
    /// 鼠标按键/滚轮回调数（含被放行的注入事件）。
    mouse_edges: AtomicU64,
    /// 鼠标移动回调数（全部放行，只作诊断）。
    mouse_moves: AtomicU64,
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

/// 把鼠标钩子消息投影成驱动的 `MOUSEEVENT` 约定：`(button_flags, button_data)`。
///
/// - 按键 → ntddmou 的 `MOUSE_*_BUTTON_DOWN/UP` 单一位，`button_data = 0`
/// - 滚轮 → `MOUSE_WHEEL` / `MOUSE_HWHEEL`，`button_data` = `mouseData` 高 16 位（**有符号**）
/// - 纯移动与其他消息 → `None`（调用方放行；与驱动"纯移动不入队"一致）
///
/// XBUTTON 的编号藏在 `mouseData` 的高 16 位（`XBUTTON1` / `XBUTTON2`），而驱动侧只用
/// `ButtonFlags` 表达，所以这里必须做一次查表。
pub fn project_mouse(msg: u32, mouse_data: u32) -> Option<(u16, i16)> {
    let hi = (mouse_data >> 16) as u16;
    let signed = hi as i16;
    let bf = match msg {
        WM_LBUTTONDOWN => MOUSE_LEFT_BUTTON_DOWN,
        WM_LBUTTONUP => MOUSE_LEFT_BUTTON_UP,
        WM_RBUTTONDOWN => MOUSE_RIGHT_BUTTON_DOWN,
        WM_RBUTTONUP => MOUSE_RIGHT_BUTTON_UP,
        WM_MBUTTONDOWN => MOUSE_MIDDLE_BUTTON_DOWN,
        WM_MBUTTONUP => MOUSE_MIDDLE_BUTTON_UP,
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let down = msg == WM_XBUTTONDOWN;
            match (hi as u32, down) {
                (h, true) if h == XBUTTON1 => MOUSE_BUTTON_4_DOWN,
                (h, true) if h == XBUTTON2 => MOUSE_BUTTON_5_DOWN,
                (h, false) if h == XBUTTON1 => MOUSE_BUTTON_4_UP,
                (h, false) if h == XBUTTON2 => MOUSE_BUTTON_5_UP,
                _ => return None,
            }
        }
        WM_MOUSEWHEEL => return Some((MOUSE_WHEEL, signed)),
        WM_MOUSEHWHEEL => return Some((MOUSE_HWHEEL, signed)),
        _ => return None,
    };
    Some((bf, 0))
}

// ── 钩子回调 ──

unsafe extern "system" fn kb_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
        // 存活对账的基准必须是"**我们被调用的时刻**"，而不是"我们成功入队的时刻"。
        // 实测踩过：用户只动鼠标、不打字时，键盘钩子不更新这个值，而 GetLastInputInfo 会因
        // 鼠标活动推进 → 误报"钩子失活"（smoke 测试里 mouse_moves=722 却打出 WARN）。
        // GetTickCount 读的是 KUSER_SHARED_DATA，无系统调用，每帧刷新开销可忽略。
        sh.last_cb_tick.store(GetTickCount(), Ordering::Relaxed);
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

        match sh.tx.try_send(HookEvent::Key(ev)) {
            Ok(()) => {
                sh.callbacks.fetch_add(1, Ordering::Relaxed);
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

unsafe extern "system" fn ms_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let next = |code: i32, wparam: WPARAM, lparam: LPARAM| -> LRESULT {
            CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
        };

        if code < 0 {
            return next(code, wparam, lparam);
        }
        let Some(sh) = SHARED.get() else {
            return next(code, wparam, lparam);
        };
        // 同键盘钩子：任意一次回调都刷新"最后一次回调时刻"（含纯移动与放行的注入事件）。
        sh.last_cb_tick.store(GetTickCount(), Ordering::Relaxed);
        let ms = &*(lparam as *const MSLLHOOKSTRUCT);
        let msg = wparam as u32;

        // 纯移动一律放行（与驱动一致：光标必须能动；高频事件"吞+重放"是性能灾难）。
        if msg == WM_MOUSEMOVE {
            sh.mouse_moves.fetch_add(1, Ordering::Relaxed);
            return next(code, wparam, lparam);
        }
        let Some((button_flags, button_data)) = project_mouse(msg, ms.mouseData) else {
            // 不认识的鼠标消息 —— 放行。
            return next(code, wparam, lparam);
        };
        sh.mouse_edges.fetch_add(1, Ordering::Relaxed);

        // 自注入放行（同键盘）。
        if ms.flags & LLMHF_INJECTED != 0 {
            return next(code, wparam, lparam);
        }
        if !sh.swallow.load(Ordering::Relaxed) {
            return next(code, wparam, lparam);
        }

        let ev = AnyKeyMouseEvent {
            device_id: 0,
            // 按键/滚轮包没有位移语义：与驱动保持同一取值（驱动的那两个字段也不参与判定）。
            flags: MOUSE_MOVE_RELATIVE,
            button_flags,
            button_data,
            last_x: 0,
            last_y: 0,
            extra_info: ms.dwExtraInfo as u32,
        };

        match sh.tx.try_send(HookEvent::Mouse(ev)) {
            Ok(()) => 1, // 吞掉
            Err(_) => {
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
        // hMod = NULL 对 WH_KEYBOARD_LL / WH_MOUSE_LL 是合法的（回调在当前进程内）。
        let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook_proc), std::ptr::null_mut(), 0);
        if kb.is_null() {
            let err = GetLastError();
            let _ = install_tx.send(Err(format!(
                "SetWindowsHookExW(WH_KEYBOARD_LL) failed, GetLastError={}",
                err
            )));
            return;
        }
        KB_HOOK.store(kb, Ordering::SeqCst);

        let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(ms_hook_proc), std::ptr::null_mut(), 0);
        if ms.is_null() {
            // 鼠标钩子装不上就整体失败：用户明确要鼠标拦截，静默降级会变成"以为能用"。
            let err = GetLastError();
            UnhookWindowsHookEx(kb);
            KB_HOOK.store(std::ptr::null_mut(), Ordering::SeqCst);
            let _ = install_tx.send(Err(format!(
                "SetWindowsHookExW(WH_MOUSE_LL) failed, GetLastError={}",
                err
            )));
            return;
        }
        MS_HOOK.store(ms, Ordering::SeqCst);
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

static KB_HOOK: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static MS_HOOK: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

// ── 对外类型 ──

/// 钩子存活对账结果（供 main loop 打日志）。
pub struct HookLiveness {
    pub callbacks: u64,
    /// 鼠标按键/滚轮回调数（含被放行的注入事件）——
    /// 用来区分"鼠标钩子没工作"和"键盘钩子没工作"。
    pub mouse_edges: u64,
    pub mouse_moves: u64,
    pub since_last_cb_ms: u32,
    /// 钩子安装至今的毫秒数 —— 用来判断"首次回调"来得有多快（装上≠收得到）。
    pub since_install_ms: u32,
    pub system_idle_ms: u32,
    pub passthrough: u64,
    /// false = "系统侧有输入，而本钩子长时间没有回调"——可能是钩子被摘除，
    /// 也可能是前台在提权窗口（不提权运行时收不到，属预期）。
    pub healthy: bool,
}

pub struct HookInput {
    rx: Receiver<HookEvent>,
    kb_hook: *mut std::ffi::c_void,
    ms_hook: *mut std::ffi::c_void,
}

unsafe impl Send for HookInput {}

impl HookInput {
    /// 安装全局键盘 + 鼠标低级钩子。成功后从那一刻起**所有物理按键（含鼠标按键/滚轮）
    /// 都会被吞掉**（A2 决策），由管道逐个重放；鼠标**移动**始终放行。
    pub fn install() -> Result<Self, String> {
        let (tx, rx) = sync_channel::<HookEvent>(CHANNEL_CAP);
        let (install_tx, install_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        SHARED
            .set(Shared {
                tx,
                swallow: AtomicBool::new(true),
                callbacks: AtomicU64::new(0),
                last_cb_tick: AtomicU32::new(0),
                install_tick: AtomicU32::new(0),
                passthrough: AtomicU64::new(0),
                mouse_edges: AtomicU64::new(0),
                mouse_moves: AtomicU64::new(0),
                pump_tid: AtomicU32::new(0),
            })
            .map_err(|_| "llhook backend already installed in this process".to_string())?;

        std::thread::Builder::new()
            .name("anykey-llhook-pump".into())
            .spawn(move || pump_thread(install_tx))
            .map_err(|e| format!("spawn pump thread failed: {}", e))?;

        match install_rx.recv_timeout(Duration::from_millis(INSTALL_TIMEOUT_MS)) {
            Ok(Ok(())) => {
                let kb = KB_HOOK.load(Ordering::SeqCst);
                let ms = MS_HOOK.load(Ordering::SeqCst);
                Ok(HookInput { rx, kb_hook: kb, ms_hook: ms })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(format!(
                "hook install timed out ({}ms) — pump thread not responding",
                INSTALL_TIMEOUT_MS
            )),
        }
    }

    /// 等待输入（键盘 + 鼠标）。**超时返回空 Vec**（与驱动后端同语义）——
    /// 主循环靠"返回空"这一刻推进 combo / tap-dance / defer 的定时器。
    pub fn poll_all(
        &self,
        timeout_ms: u32,
    ) -> Result<(Vec<AnyKeyInputEvent>, Vec<AnyKeyMouseEvent>), String> {
        let mut kb: Vec<AnyKeyInputEvent> = Vec::with_capacity(16);
        let mut ms: Vec<AnyKeyMouseEvent> = Vec::with_capacity(4);

        match self.rx.recv_timeout(Duration::from_millis(timeout_ms as u64)) {
            Ok(HookEvent::Key(k)) => kb.push(k),
            Ok(HookEvent::Mouse(m)) => ms.push(m),
            Err(RecvTimeoutError::Timeout) => return Ok((kb, ms)),
            Err(RecvTimeoutError::Disconnected) => {
                // 泵线程死了 → 钩子也已失效（事件会原样透传），引擎应退出而不是继续跑。
                return Err("llhook pump thread exited (channel closed)".into());
            }
        }
        while kb.len() + ms.len() < BATCH_MAX {
            match self.rx.try_recv() {
                Ok(HookEvent::Key(k)) => kb.push(k),
                Ok(HookEvent::Mouse(m)) => ms.push(m),
                Err(_) => break,
            }
        }
        Ok((kb, ms))
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
    /// "最后一次回调"含**鼠标钩子**的回调 —— 这一点是实测修正过的：只动鼠标不打字时，
    /// 若基准只看键盘入队，就会被误判成钩子失活（smoke 测试里 mouse_moves=722 打出 WARN）。
    ///
    /// 用 `GetLastInputInfo()` 而不是自己的计数器，是因为它是系统级、**不经过本钩子**
    /// 的独立见证（实测验证过，见 tools/probe_elev_hook2.py）。
    pub fn liveness(&self) -> HookLiveness {
        let now = unsafe { GetTickCount() };
        let (callbacks, mouse_edges, mouse_moves, passthrough, baseline, install) =
            match SHARED.get() {
                Some(sh) => {
                    let install = sh.install_tick.load(Ordering::Relaxed);
                    let last = sh.last_cb_tick.load(Ordering::Relaxed);
                    let base = if last != 0 { last } else { install };
                    (
                        sh.callbacks.load(Ordering::Relaxed),
                        sh.mouse_edges.load(Ordering::Relaxed),
                        sh.mouse_moves.load(Ordering::Relaxed),
                        sh.passthrough.load(Ordering::Relaxed),
                        base,
                        install,
                    )
                }
                None => (0, 0, 0, 0, now, now),
            };
        let since_last_cb_ms = now.wrapping_sub(baseline);
        let system_idle_ms = system_idle_ms(now);
        // 系统侧输入的"年龄"比我们的静默时长更小 → 那次输入没经过本钩子。
        let missed = system_idle_ms < since_last_cb_ms;
        HookLiveness {
            callbacks,
            mouse_edges,
            mouse_moves,
            since_last_cb_ms,
            since_install_ms: now.wrapping_sub(install),
            system_idle_ms,
            passthrough,
            healthy: !(missed && since_last_cb_ms > HOOK_SILENT_LIMIT_MS),
        }
    }
}

impl Drop for HookInput {
    fn drop(&mut self) {
        // 顺序：先撤闸门（停止吞事件，在途回调立即变透传）→ 卸钩子 → 让泵线程退出。
        if let Some(sh) = SHARED.get() {
            sh.swallow.store(false, Ordering::SeqCst);
        }
        if !self.ms_hook.is_null() {
            unsafe { UnhookWindowsHookEx(self.ms_hook) };
            MS_HOOK.store(std::ptr::null_mut(), Ordering::SeqCst);
        }
        if !self.kb_hook.is_null() {
            unsafe { UnhookWindowsHookEx(self.kb_hook) };
            KB_HOOK.store(std::ptr::null_mut(), Ordering::SeqCst);
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

    #[test]
    fn project_mouse_three_buttons() {
        assert_eq!(project_mouse(WM_LBUTTONDOWN, 0), Some((MOUSE_LEFT_BUTTON_DOWN, 0)));
        assert_eq!(project_mouse(WM_LBUTTONUP, 0), Some((MOUSE_LEFT_BUTTON_UP, 0)));
        assert_eq!(project_mouse(WM_RBUTTONDOWN, 0), Some((MOUSE_RIGHT_BUTTON_DOWN, 0)));
        assert_eq!(project_mouse(WM_RBUTTONUP, 0), Some((MOUSE_RIGHT_BUTTON_UP, 0)));
        assert_eq!(project_mouse(WM_MBUTTONDOWN, 0), Some((MOUSE_MIDDLE_BUTTON_DOWN, 0)));
        assert_eq!(project_mouse(WM_MBUTTONUP, 0), Some((MOUSE_MIDDLE_BUTTON_UP, 0)));
    }

    #[test]
    fn project_mouse_xbuttons_carry_id_in_high_word() {
        // XBUTTON 编号在 mouseData 的高 16 位：1 = XBUTTON1 → 驱动 4 号键，2 = XBUTTON2 → 5 号键
        let x1 = (XBUTTON1 as u32) << 16;
        let x2 = (XBUTTON2 as u32) << 16;
        assert_eq!(project_mouse(WM_XBUTTONDOWN, x1), Some((MOUSE_BUTTON_4_DOWN, 0)));
        assert_eq!(project_mouse(WM_XBUTTONUP, x1), Some((MOUSE_BUTTON_4_UP, 0)));
        assert_eq!(project_mouse(WM_XBUTTONDOWN, x2), Some((MOUSE_BUTTON_5_DOWN, 0)));
        assert_eq!(project_mouse(WM_XBUTTONUP, x2), Some((MOUSE_BUTTON_5_UP, 0)));
        // 未知编号 → 不投影（放行）
        assert_eq!(project_mouse(WM_XBUTTONDOWN, 7 << 16), None);
    }

    #[test]
    fn project_mouse_wheel_keeps_signed_delta() {
        // 上滚 120
        assert_eq!(project_mouse(WM_MOUSEWHEEL, 120u32 << 16), Some((MOUSE_WHEEL, 120)));
        // 下滚 -120（高 16 位按有符号解释）
        assert_eq!(project_mouse(WM_MOUSEWHEEL, 0xFF88u32 << 16), Some((MOUSE_WHEEL, -120)));
        assert_eq!(project_mouse(WM_MOUSEHWHEEL, 120u32 << 16), Some((MOUSE_HWHEEL, 120)));
        assert_eq!(project_mouse(WM_MOUSEHWHEEL, 0xFF88u32 << 16), Some((MOUSE_HWHEEL, -120)));
    }

    #[test]
    fn project_mouse_ignores_pure_move_and_unknown() {
        // 纯移动不入管道 —— 与驱动 "Pure movement (ButtonFlags==0): forward to system" 一致
        assert_eq!(project_mouse(WM_MOUSEMOVE, 0), None);
        assert_eq!(project_mouse(0x0999, 0), None);
    }
}
