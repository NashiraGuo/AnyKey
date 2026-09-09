#![windows_subsystem = "windows"]

//! AnyKey 系统托盘 —— Rust 版（替代 tray/main.py）
//! 职责：引擎生命周期管理、IPC 服务（127.0.0.1:19527）、开机自启、状态监控。
//!
//! 线程模型：
//!   - 主线程：隐藏窗口 + 托盘 + 消息循环（图标/菜单/睡眠唤醒/TaskbarCreated）
//!   - IPC 线程：TCP accept + 命令处理（crate::ipc）
//!   - 监控线程：每 3s 检查引擎存活（crate::engine::spawn_monitor）
//!   - 启动线程：0.5s 后自动拉起引擎

mod app;
mod engine;
mod icon;
mod ipc;
mod paths;
mod registry;
mod tray;

use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use app::Shared;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Threading::CreateMutexW;

/// 单实例互斥（与 Python 版一致：CreateMutexW + ERROR_ALREADY_EXISTS=183）
fn ensure_single_instance() -> bool {
    let name: Vec<u16> = "AnyKey_TrayInstance".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _h = CreateMutexW(ptr::null(), 1, name.as_ptr());
        GetLastError() != 183
    }
}

fn main() {
    if !ensure_single_instance() {
        return;
    }

    let base_dir = paths::find_base_dir();
    let debug = app::load_debug_flag(&base_dir);

    let shared = Shared::new(base_dir.clone(), debug);

    // [调试] 设置 ANYKEY_DUMP_ICON=1 启动时落盘图标到 base_dir/anykey-tray-icon.bmp（用于任务栏图标对比排查）
    if std::env::var_os("ANYKEY_DUMP_ICON").is_some() {
        let _ = icon::dump_to_bmp(false, &base_dir.join("anykey-tray-icon.bmp"));
    }

    // 清理残留引擎进程（启动时）
    engine::cleanup_orphaned();

    // 启动 IPC 服务端（后台线程）
    ipc::spawn(Arc::clone(&shared));

    // 引擎存活监控（后台线程）
    engine::spawn_monitor(Arc::clone(&shared));

    // 0.5s 后自动运行引擎（等 IPC 服务端就绪；失败不置暂停态，与 Python 版一致）
    let s_auto = Arc::clone(&shared);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        if !engine::start(&s_auto) {
            // 引擎文件缺失或启动即退出：通过 IPC status 暴露给 GUI
            s_auto.paused.store(true, Ordering::SeqCst);
        }
    });

    // 主线程：窗口 + 托盘 + 消息循环（阻塞至退出）
    tray::run(Arc::clone(&shared));

    // 退出清理
    shared.running.store(false, Ordering::SeqCst);
    engine::kill(&shared);
}
