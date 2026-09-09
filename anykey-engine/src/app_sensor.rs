//! app_sensor — foreground 应用感知：SetWinEventHook（事件驱动，主路径）
//! + 500ms watchdog（GetForegroundWindow 一致性兜底，防通知丢失）。
//!
//! 设计要点：
//! - SetWinEventHook 的 out-of-context 回调依赖「安装线程的 Windows 消息循环」
//!   派发（系统以消息投递给安装线程，线程 pump 消息时才调用回调）。因此 hook 必须
//!   挂在专用线程跑消息循环——若放在引擎主循环（poll 阻塞、无 GetMessage），
//!   foreground 切换事件永远不触发。
//! - WinEvent 通知只是「可能变化」的触发器：回调里只置 DIRTY 标志，绝不做
//!   OpenProcess / QueryFullProcessImageName 等重活。真正的 resolve 统一放在消息
//!   循环之后，用 GetForegroundWindow() 重新确认实际前台窗口，避免事件处理过程中
//!   窗口再次变化造成状态过期。
//! - 500ms watchdog 由 MsgWaitForMultipleObjectsEx 超时驱动（同一条 hook 线程，
//!   不开第二条线程）：比对实际 HWND 与缓存 HWND，不一致才重新 resolve。
//! - 结果经「最新值槽」Mutex<Option<ForegroundInfo>> 交给主循环：latest-wins，
//!   不阻塞、不堆积、不丢最新状态。
//! - 按键热路径绝不调用任何窗口/进程查询——主循环只消费 ForegroundInfo。

/// 前台窗口快照：HWND（isize，裸指针非 Send）+ PID + 进程名 + 单调递增 generation。
#[derive(Debug, Clone)]
pub struct ForegroundInfo {
    pub hwnd: isize,
    pub pid: u32,
    pub process: String,
    pub generation: u64,
}

#[cfg(feature = "filter-driver")]
mod inner {
    use super::ForegroundInfo;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::thread::JoinHandle;

    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::Accessibility::SetWinEventHook;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetForegroundWindow, GetWindowThreadProcessId,
        MsgWaitForMultipleObjectsEx, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        QS_ALLINPUT, WM_QUIT,
    };

    const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
    /// watchdog 间隔：500ms 一次 GetForegroundWindow，开销极低。
    const WATCHDOG_INTERVAL_MS: u32 = 500;

    /// 事件回调只置位（脏标记），不做重活。
    static DIRTY: AtomicBool = AtomicBool::new(false);
    /// 最新前台状态槽：主循环 try_recv 取走（take），latest-wins。
    static STATE: OnceLock<Mutex<Option<ForegroundInfo>>> = OnceLock::new();

    pub struct Sensor {
        _join: JoinHandle<()>,
    }

    impl Sensor {
        pub fn new() -> Self {
            STATE.get_or_init(|| Mutex::new(None));
            DIRTY.store(true, Ordering::SeqCst); // 启动即 resolve 一次
            let handle = std::thread::spawn(run_loop);
            Sensor { _join: handle }
        }

        pub fn try_recv(&self) -> Option<ForegroundInfo> {
            STATE.get().and_then(|m| m.lock().unwrap().take())
        }
    }

    fn run_loop() {
        unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                std::ptr::null_mut(),
                Some(on_fg),
                0,
                0,
                0x0000,
            );
        }

        let mut cached_hwnd: isize = 0;
        let mut generation: u64 = 0;
        // 初始 resolve：不等 500ms，启动即拿到当前前台。
        check_and_emit(&mut cached_hwnd, &mut generation);

        let mut msg: MSG = unsafe { std::mem::zeroed() };
        loop {
            // 等消息或 500ms 超时（watchdog）。
            let res = unsafe {
                MsgWaitForMultipleObjectsEx(0, std::ptr::null(), WATCHDOG_INTERVAL_MS, QS_ALLINPUT, 0)
            };

            if res == WAIT_OBJECT_0 {
                // 有消息：全部 pump；WinEvent callback 在此触发（只置 DIRTY）。
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

            // 事件（DIRTY）或 watchdog 超时 → 一致性检查。
            if DIRTY.swap(false, Ordering::SeqCst) || res == WAIT_TIMEOUT {
                check_and_emit(&mut cached_hwnd, &mut generation);
            }
        }
    }

    /// 回调只置脏标记——通知仅作「可能变化」的触发信号。
    unsafe extern "system" fn on_fg(
        _h: *mut c_void, _e: u32, _w: *mut c_void, _o: i32, _c: i32, _t: u32, _m: u32,
    ) {
        DIRTY.store(true, Ordering::SeqCst);
    }

    /// 读当前前台窗口；仅当 HWND 与缓存不同才做完整 resolve，并写入最新值槽。
    fn check_and_emit(cached_hwnd: &mut isize, generation: &mut u64) {
        if let Some(mut info) = resolve_if_changed(*cached_hwnd) {
            *cached_hwnd = info.hwnd;
            *generation += 1;
            info.generation = *generation;
            if let Some(slot) = STATE.get() {
                *slot.lock().unwrap() = Some(info); // latest-wins，覆盖旧值
            }
        }
    }

    /// 实际 resolve：HWND 变了才做 OpenProcess + QueryFullProcessImageName。
    fn resolve_if_changed(cached_hwnd: isize) -> Option<ForegroundInfo> {
        unsafe {
            let raw = GetForegroundWindow();
            let hwnd = raw as isize;
            if hwnd == cached_hwnd {
                return None;
            }
            if raw.is_null() {
                return Some(ForegroundInfo { hwnd: 0, pid: 0, process: String::new(), generation: 0 });
            }

            let mut pid: u32 = 0;
            GetWindowThreadProcessId(raw, &mut pid);
            if pid == 0 {
                return Some(ForegroundInfo { hwnd, pid: 0, process: String::new(), generation: 0 });
            }

            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return Some(ForegroundInfo { hwnd, pid, process: String::new(), generation: 0 });
            }

            let mut buf = [0u16; 260];
            let mut len = buf.len() as u32;
            let process = if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0 {
                let path = String::from_utf16_lossy(&buf[..len as usize]);
                path.rsplit('\\').next().unwrap_or("").to_string()
            } else {
                String::new()
            };
            CloseHandle(h);
            Some(ForegroundInfo { hwnd, pid, process, generation: 0 })
        }
    }
}

#[cfg(feature = "filter-driver")]
pub use inner::Sensor as AppSensor;

#[cfg(not(feature = "filter-driver"))]
pub struct AppSensor;

#[cfg(not(feature = "filter-driver"))]
impl AppSensor {
    pub fn new() -> Self { AppSensor }
    pub fn try_recv(&self) -> Option<ForegroundInfo> { None }
}
