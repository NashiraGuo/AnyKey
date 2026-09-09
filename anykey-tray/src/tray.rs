//! 托盘主窗口 —— 隐藏窗口 + Shell_NotifyIconW 托盘 + 右键菜单 + 消息循环。
//! 对应原 tray/main.py 的 pystray.Icon + _tray_recovery_loop（合并为一个窗口：
//! 同时监听托盘回调、TaskbarCreated、睡眠唤醒、60s 健康检查）。

use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, OnceLock};

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NOTIFYICONDATAW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::{self, Shared, WM_APP_QUIT, WM_APP_UPDATE_ICON, WM_TRAY};

// ── 菜单命令 ID ──
const ID_OPEN_GUI: usize = 1;
const ID_TOGGLE_PAUSE: usize = 2;
const ID_RELOAD: usize = 3;
const ID_TOGGLE_AUTOSTART: usize = 4;
const ID_TOGGLE_DEBUG: usize = 5;
const ID_QUIT: usize = 6;

// ── 自注册消息 / 定时器（WM_POWERBROADCAST 等常量直接内联，避免依赖额外 feature） ──
const WM_POWERBROADCAST: u32 = 0x0218;
const PBT_APMRESUMEAUTOMATIC: u32 = 0x0012;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_LBUTTONUP: u32 = 0x0202;
const TIMER_HEALTH: usize = 1;
const TRAY_ID: u32 = 1;

static ONCE: OnceLock<Arc<Shared>> = OnceLock::new();
/// 当前托盘图标句柄（HICON 非 Send，用 AtomicIsize 存指针）
static CURRENT_ICON: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    match msg {
        WM_TRAY => {
            let action = (lparam as u32) & 0xFFFF;
            match action {
                WM_RBUTTONUP => show_menu(hwnd),
                WM_LBUTTONUP => {
                    if let Some(s) = ONCE.get() {
                        app::open_gui(s);
                    }
                }
                _ => {}
            }
            0
        }
        WM_APP_UPDATE_ICON => {
            refresh_tray_icon();
            0
        }
        WM_APP_QUIT => {
            PostQuitMessage(0);
            0
        }
        WM_POWERBROADCAST => {
            if wparam as u32 == PBT_APMRESUMEAUTOMATIC {
                refresh_tray_icon();
            }
            0
        }
        msg if Some(msg) == TASKBAR_CREATED.get().copied() => {
            refresh_tray_icon();
            0
        }
        WM_TIMER if wparam == TIMER_HEALTH => {
            // 60s 健康检查：explorer 异常时兜底重建图标
            refresh_tray_icon();
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 构建 NOTIFYICONDATAW（填充 hwnd/uID/回调消息/tooltip）
unsafe fn build_nid(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = mem::zeroed();
    nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = icon;
    let tip: Vec<u16> = "AnyKey - 任意键".encode_utf16().collect();
    for (dst, src) in nid.szTip.iter_mut().zip(tip.iter()) {
        *dst = *src;
    }
    nid
}

/// 重建托盘图标（状态切换 / TaskbarCreated / 睡眠唤醒 / 定时兜底）
fn refresh_tray_icon() {
    let Some(shared) = ONCE.get() else { return };
    let hwnd = shared.hwnd();
    if hwnd.is_null() {
        return;
    }
    let paused = shared.paused.load(Ordering::SeqCst);
    let icon = crate::icon::create(paused);
    unsafe {
        let nid = build_nid(hwnd, icon);
        // 先 MODIFY（图标已存在）；失败（explorer 重启后）则 ADD
        if Shell_NotifyIconW(NIM_MODIFY, &nid) == 0 {
            Shell_NotifyIconW(NIM_ADD, &nid);
        }
    }
    // 替换全局图标（destroy 旧的，避免泄漏）
    let old = CURRENT_ICON.swap(icon as isize, Ordering::SeqCst);
    if old != 0 && old != icon as isize {
        unsafe { DestroyIcon(old as HICON) };
    }
}

/// 构建右键菜单（每次点击时按当前状态生成，无需“重建菜单”）
unsafe fn build_menu(shared: &Shared) -> HMENU {
    let hmenu = CreatePopupMenu();
    let add = |text: &str, id: usize, checked: bool| {
        let text = to_wide(text);
        let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
        AppendMenuW(hmenu, flags, id, text.as_ptr());
    };
    add("打开主窗口", ID_OPEN_GUI, false);
    AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
    let pause_text = if shared.paused.load(Ordering::SeqCst) {
        "恢复任意键"
    } else {
        "暂停任意键"
    };
    add(pause_text, ID_TOGGLE_PAUSE, false);
    add("重载设置", ID_RELOAD, false);
    AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
    add("开机自启", ID_TOGGLE_AUTOSTART, crate::registry::is_enabled());
    add("调试模式", ID_TOGGLE_DEBUG, *shared.debug.lock().unwrap());
    AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
    add("退出", ID_QUIT, false);
    hmenu
}

/// 弹出右键菜单（TPM_RETURNCMD 同步返回选中的命令）
unsafe fn show_menu(hwnd: HWND) {
    let Some(shared) = ONCE.get() else { return };
    let hmenu = build_menu(shared);
    SetForegroundWindow(hwnd);
    let mut pt: POINT = mem::zeroed();
    GetCursorPos(&mut pt);
    let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON;
    let cmd = TrackPopupMenu(hmenu, flags, pt.x, pt.y, 0, hwnd, ptr::null());
    DestroyMenu(hmenu);
    if cmd != 0 {
        handle_menu_command(shared, cmd as usize);
    }
}

fn handle_menu_command(shared: &Arc<Shared>, cmd: usize) {
    match cmd {
        ID_OPEN_GUI => app::open_gui(shared),
        ID_TOGGLE_PAUSE => app::toggle_pause(shared),
        ID_RELOAD => app::do_reload(shared),
        ID_TOGGLE_AUTOSTART => app::toggle_autostart(shared),
        ID_TOGGLE_DEBUG => app::toggle_debug(shared),
        ID_QUIT => app::quit_all(shared),
        _ => {}
    }
}

/// 主入口：注册窗口类 → 创建隐藏窗口 → 添加托盘 → 消息循环 → 清理
pub fn run(shared: Arc<Shared>) {
    unsafe {
        let hinst = GetModuleHandleW(ptr::null());
        let class_name = to_wide("AnyKeyTrayWnd");
        let empty = to_wide("");

        let wc = WNDCLASSEXW {
            cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: ptr::null_mut(),
        };
        if RegisterClassExW(&wc) == 0 {
            eprintln!("[tray] RegisterClassExW 失败");
            return;
        }

        let hwnd = CreateWindowExW(
            0x80, // WS_EX_TOOLWINDOW：不出现于任务栏/Alt-Tab
            class_name.as_ptr(),
            empty.as_ptr(),
            0x8000_0000, // WS_POPUP：不可见顶层窗口
            0, 0, 0, 0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            ptr::null(),
        );
        if hwnd.is_null() {
            eprintln!("[tray] CreateWindowExW 失败");
            return;
        }

        let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(to_wide("TaskbarCreated").as_ptr()));
        let _ = ONCE.set(shared);
        let shared = ONCE.get().unwrap();
        shared.hwnd.store(hwnd as isize, Ordering::SeqCst);

        // 初始托盘图标
        let icon = crate::icon::create(shared.paused.load(Ordering::SeqCst));
        {
            let nid = build_nid(hwnd, icon);
            Shell_NotifyIconW(NIM_ADD, &nid);
        }
        CURRENT_ICON.store(icon as isize, Ordering::SeqCst);

        // 60s 健康检查定时器
        SetTimer(hwnd, TIMER_HEALTH, 60_000, None);

        // 消息循环
        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // 清理：删除托盘图标 + 释放图标 + 销毁窗口
        let nid = build_nid(hwnd, ptr::null_mut());
        Shell_NotifyIconW(NIM_DELETE, &nid);
        let icon = CURRENT_ICON.swap(0, Ordering::SeqCst);
        if icon != 0 {
            DestroyIcon(icon as HICON);
        }
        DestroyWindow(hwnd);
        UnregisterClassW(class_name.as_ptr(), hinst);
    }
}
