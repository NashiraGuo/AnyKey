//! 路径定位 —— 与 lib/config.py 的路径约定保持一致：
//! - 打包：exe 位于安装根目录（dist/AnyKey/anykey-tray.exe），同目录有
//!   anykey_config.json / anykey-gui.exe / engines/rust/anykey-engine.exe
//! - 开发：exe 位于 anykey-tray/target/{release,debug}/，向上逐级找到
//!   项目根（AnyKey/）的 anykey_config.json

use std::path::{Path, PathBuf};

/// 从当前 exe 所在目录向上查找安装根目录（存在 anykey_config.json 的目录）
pub fn find_base_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    // 向上最多 4 级寻找 anykey_config.json（覆盖 target/release → anykey-tray → AnyKey）
    let mut dir: Option<&Path> = Some(&exe_dir);
    for _ in 0..4 {
        if let Some(d) = dir {
            if d.join("anykey_config.json").is_file() {
                return d.to_path_buf();
            }
            dir = d.parent();
        }
    }
    // 兜底：exe 所在目录
    exe_dir
}

/// 配置文件路径
pub fn config_path(base: &Path) -> PathBuf {
    base.join("anykey_config.json")
}

/// 引擎 exe 路径：打包优先 engines/rust/，开发回退 target 编译目录
pub fn find_engine_exe(base: &Path) -> Option<PathBuf> {
    for rel in [
        "engines/rust/anykey-engine.exe",
        "anykey-engine/target/release/anykey-engine.exe",
        "anykey-engine/target/debug/anykey-engine.exe",
    ] {
        let p = base.join(rel);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// GUI 启动方式：优先 anykey-gui.exe；开发模式回退 python 运行 gui/main.py
/// 返回 (程序, 参数列表)
pub fn find_gui_launch(base: &Path) -> Option<(PathBuf, Vec<String>)> {
    let gui_exe = base.join("anykey-gui.exe");
    if gui_exe.is_file() {
        return Some((gui_exe, vec![]));
    }

    // 开发模式：venv 优先，其次 PATH 中的 pythonw / python / py
    let script = base.join("gui").join("main.py");
    if script.is_file() {
        for py in [
            base.join("venv").join("Scripts").join("pythonw.exe"),
            base.join("venv").join("Scripts").join("python.exe"),
            base.join("venv").join("Scripts").join("pythonw"),
            PathBuf::from("pythonw"),
            PathBuf::from("python"),
            PathBuf::from("py"),
        ] {
            if py.is_file() {
                return Some((py, vec![script.to_string_lossy().into_owned()]));
            }
        }
        // PATH 兜底：不检查存在性，交给 Command 解析
        return Some((PathBuf::from("python"), vec![script.to_string_lossy().into_owned()]));
    }
    None
}
