//! 引擎进程管理 —— 对应原 tray/main.py 的 _start_rust/_kill_engine/_is_engine_running
//! 与监控线程。启动方式与 Python 版一致：
//!   taskkill 残留 → 启动 [engine_exe, config.json, (--debug)] DETACHED + NO_WINDOW
//!   → 0.5s 后确认存活 → 记录 pid。

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};

use crate::app::Shared;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

/// 清除残留 anykey-engine.exe 进程（taskkill，与 Python 版一致）
pub fn cleanup_orphaned() {
    let _ = Command::new("taskkill")
        .args(["/f", "/im", "anykey-engine.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

/// 启动 Rust 引擎；成功返回 true 并写入 shared.engine_pid
pub fn start(shared: &Arc<Shared>) -> bool {
    let engine_exe = match crate::paths::find_engine_exe(&shared.base_dir) {
        Some(p) => p,
        None => {
            eprintln!("[tray] Rust 引擎未找到");
            return false;
        }
    };

    // 杀残留进程（Python 版 taskkill + 0.3s 等待）
    cleanup_orphaned();
    std::thread::sleep(Duration::from_millis(300));

    let config_path = crate::paths::config_path(&shared.base_dir);
    let mut cmd = Command::new(&engine_exe);
    cmd.arg(&config_path);
    if *shared.debug.lock().unwrap() {
        cmd.arg("--debug");
    }
    cmd.current_dir(engine_exe.parent().unwrap_or(Path::new(".")))
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[tray] 引擎启动失败: {}", e);
            return false;
        }
    };
    let pid = child.id();

    // 0.5s 后确认进程仍存活（与 Python 版 poll 检查一致）
    std::thread::sleep(Duration::from_millis(500));
    match child.try_wait() {
        Ok(Some(status)) => {
            eprintln!("[tray] 引擎启动后立即退出，返回码: {}", status.code().unwrap_or(-1));
            return false;
        }
        Ok(None) => {
            *shared.engine_pid.lock().unwrap() = pid;
            true
        }
        Err(e) => {
            eprintln!("[tray] 引擎状态检查失败: {}", e);
            false
        }
    }
}

/// 进程是否存活
fn is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        let h: HANDLE = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        CloseHandle(h);
        true
    }
}

/// 杀引擎：OpenProcess(PROCESS_TERMINATE) + TerminateProcess + 兜底 taskkill
pub fn kill(shared: &Arc<Shared>) {
    let pid = *shared.engine_pid.lock().unwrap();
    if pid != 0 {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !h.is_null() {
                let _ = TerminateProcess(h, 0);
                CloseHandle(h);
            }
        }
        *shared.engine_pid.lock().unwrap() = 0;
    }
    cleanup_orphaned();
}

/// 启动引擎监控线程：每 3s 检查，意外退出 → paused=true + 更新图标 + 推送事件
pub fn spawn_monitor(shared: Arc<Shared>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3));
        if !shared.running.load(Ordering::Relaxed) {
            break;
        }
        let pid = *shared.engine_pid.lock().unwrap();
        if pid != 0 && !is_running(pid) {
            *shared.engine_pid.lock().unwrap() = 0;
            if !shared.paused.swap(true, Ordering::SeqCst) {
                crate::app::notify_state_changed(&shared, "引擎进程已意外退出");
            }
            // 更新托盘图标（走主线程）
            crate::app::post_update_icon(&shared);
        }
    });
}

/// 引擎是否在运行（供 status 查询）
pub fn running(shared: &Arc<Shared>) -> bool {
    let pid = *shared.engine_pid.lock().unwrap();
    pid != 0 && is_running(pid)
}
