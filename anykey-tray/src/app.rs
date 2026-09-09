//! 应用层 —— 共享状态 + IPC 命令分发 + 引擎生命周期操作。
//! 对应原 tray/main.py 的 AnyKeyTray 类（除窗口/托盘 UI 外全部逻辑）。

use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, SetForegroundWindow, ShowWindow};

use crate::engine;

/// 主线程自定义消息
pub const WM_APP_UPDATE_ICON: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;
pub const WM_APP_QUIT: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 2;
/// 托盘回调消息（Shell_NotifyIconW 的 uCallbackMessage）
pub const WM_TRAY: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 10;

/// 跨线程共享状态
pub struct Shared {
    pub base_dir: PathBuf,
    pub running: AtomicBool,
    pub paused: AtomicBool,
    pub debug: Mutex<bool>,
    pub engine_pid: Mutex<u32>,
    /// 托盘主窗口句柄（消息循环创建后写入，供 IPC 线程触发图标刷新）
    pub hwnd: AtomicIsize,
    /// IPC 事件推送目标客户端
    pub client: Mutex<Option<TcpStream>>,
}

impl Shared {
    pub fn new(base_dir: PathBuf, debug: bool) -> Arc<Self> {
        Arc::new(Shared {
            base_dir,
            running: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            debug: Mutex::new(debug),
            engine_pid: Mutex::new(0),
            hwnd: AtomicIsize::new(0),
            client: Mutex::new(None),
        })
    }

    pub fn hwnd(&self) -> windows_sys::Win32::Foundation::HWND {
        self.hwnd.load(Ordering::SeqCst) as *mut std::ffi::c_void
    }
}

/// 请求主线程刷新托盘图标（IPC/监控线程调用）
pub fn post_update_icon(shared: &Arc<Shared>) {
    let hwnd = shared.hwnd();
    if !hwnd.is_null() {
        unsafe {
            PostMessageW(hwnd, WM_APP_UPDATE_ICON, 0, 0);
        }
    }
}

/// 推送 state_changed 事件 + 刷新图标
pub fn notify_state_changed(shared: &Arc<Shared>, _reason: &str) {
    let running = engine::running(shared);
    let paused = shared.paused.load(Ordering::SeqCst);
    crate::ipc::send_event(
        shared,
        json!({"event": "state_changed", "running": running, "paused": paused}),
    );
    post_update_icon(shared);
}

/// IPC 命令分发；返回 Some(附加响应字段) 表示命令已处理，None 表示未知命令
pub fn handle_command(shared: &Arc<Shared>, cmd: &str) -> Option<Value> {
    match cmd {
        "pause" => {
            do_pause(shared);
            Some(json!({"paused": true}))
        }
        "resume" => {
            do_resume(shared);
            Some(json!({"paused": false}))
        }
        "reload" => {
            do_reload(shared);
            Some(json!({"running": engine::running(shared)}))
        }
        "status" => Some(json!({
            "running": engine::running(shared),
            "paused": shared.paused.load(Ordering::SeqCst),
            "debug": *shared.debug.lock().unwrap(),
        })),
        "quit" => {
            // 与 Python 版一致：独立线程执行级联退出，响应立即返回
            let s = Arc::clone(shared);
            std::thread::spawn(move || quit_all(&s));
            Some(json!({}))
        }
        "subscribe" | "ping" => Some(json!({})), // Python 版对这两个命令返回空 dict
        _ => None,
    }
}

/// 暂停：杀引擎 → paused=true → 事件+图标
pub fn do_pause(shared: &Arc<Shared>) {
    engine::kill(shared);
    shared.paused.store(true, Ordering::SeqCst);
    notify_state_changed(shared, "paused");
}

/// 恢复：启动引擎 → 成功则 paused=false → 事件+图标
pub fn do_resume(shared: &Arc<Shared>) {
    if engine::start(shared) {
        shared.paused.store(false, Ordering::SeqCst);
        println!("[tray] 引擎已恢复");
    } else {
        println!("[tray] 引擎启动失败");
    }
    notify_state_changed(shared, "resumed");
}

/// 重载：读配置重启引擎 → 事件+图标
pub fn do_reload(shared: &Arc<Shared>) {
    if engine::start(shared) {
        shared.paused.store(false, Ordering::SeqCst);
        println!("[tray] 引擎已重载");
    } else {
        println!("[tray] 引擎重载失败");
    }
    notify_state_changed(shared, "reloaded");
}

/// 级联退出：tray_shutdown 事件 → 0.3s → 杀引擎 → 退出消息循环
pub fn quit_all(shared: &Arc<Shared>) {
    crate::ipc::send_event(shared, json!({"event": "tray_shutdown"}));
    std::thread::sleep(Duration::from_millis(300));
    engine::kill(shared);
    let hwnd = shared.hwnd();
    if !hwnd.is_null() {
        unsafe {
            PostMessageW(hwnd, WM_APP_QUIT, 0, 0);
        }
    }
}

/// 切换暂停/恢复（菜单）
pub fn toggle_pause(shared: &Arc<Shared>) {
    if shared.paused.load(Ordering::SeqCst) {
        do_resume(shared);
    } else {
        do_pause(shared);
    }
}

/// 切换调试模式：改配置 → 重载引擎
pub fn toggle_debug(shared: &Arc<Shared>) {
    let mut debug = shared.debug.lock().unwrap();
    *debug = !*debug;
    save_debug_flag(shared, *debug);
    drop(debug);
    do_reload(shared);
}

fn save_debug_flag(shared: &Arc<Shared>, enabled: bool) {
    let path = crate::paths::config_path(&shared.base_dir);
    let mut cfg: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    cfg["debug_enabled"] = json!(enabled);
    if let Ok(text) = serde_json::to_string_pretty(&cfg) {
        let _ = std::fs::write(&path, text);
    }
}

/// 读取配置中的 debug_enabled
pub fn load_debug_flag(base_dir: &PathBuf) -> bool {
    let path = crate::paths::config_path(base_dir);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("debug_enabled").and_then(|d| d.as_bool()))
        .unwrap_or(false)
}

/// 切换开机自启
pub fn toggle_autostart(_shared: &Arc<Shared>) {
    if crate::registry::is_enabled() {
        crate::registry::disable();
        println!("[tray] 开机自启已关闭");
    } else {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("anykey-tray.exe"));
        match crate::registry::enable(&exe) {
            Ok(()) => println!("[tray] 开机自启已开启: {:?}", exe),
            Err(e) => println!("[tray] 设置开机自启失败: {}", e),
        }
    }
}

/// 打开主窗口：已运行则激活，否则启动 anykey-gui.exe
pub fn open_gui(shared: &Arc<Shared>) {
    let title: Vec<u16> = "AnyKey - 任意键 配置管理器".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, 9); // SW_RESTORE
            SetForegroundWindow(hwnd);
            return;
        }
    }

    let Some((program, args)) = crate::paths::find_gui_launch(&shared.base_dir) else {
        println!("[tray] 未找到 anykey-gui.exe");
        return;
    };

    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new(&program);
    cmd.args(&args)
        .current_dir(&shared.base_dir)
        .creation_flags(0x0000_0008 | 0x0800_0000); // DETACHED_PROCESS | CREATE_NO_WINDOW
    if let Err(e) = cmd.spawn() {
        println!("[tray] 启动 GUI 失败: {}", e);
    }
}
