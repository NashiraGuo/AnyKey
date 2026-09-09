//! 开机自启 —— 注册表 HKCU\Software\Microsoft\Windows\CurrentVersion\Run，值名 AnyKey
//! （对应原 tray/main.py 的 winreg 逻辑；Rust 版一律注册自身 exe 路径）

use std::path::Path;

use windows_sys::Win32::System::Registry::*;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "AnyKey";

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// 开机自启是否已开启
pub fn is_enabled() -> bool {
    let key = to_wide(RUN_KEY);
    let name = to_wide(VALUE_NAME);
    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return false;
        }
        let mut buf = [0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let mut kind = 0u32;
        let rc = RegQueryValueExW(
            hkey,
            name.as_ptr(),
            std::ptr::null(),
            &mut kind,
            buf.as_mut_ptr() as *mut u8,
            &mut size,
        );
        RegCloseKey(hkey);
        rc == 0
    }
}

/// 注册自身 exe 到开机自启（值带引号，避免路径含空格被拆散）
pub fn enable(exe_path: &Path) -> std::io::Result<()> {
    let key = to_wide(RUN_KEY);
    let name = to_wide(VALUE_NAME);
    let cmd = format!("\"{}\"", exe_path.to_string_lossy());
    let value = to_wide(&cmd);
    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        let rc = RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_SET_VALUE, &mut hkey);
        if rc != 0 {
            return Err(std::io::Error::from_raw_os_error(rc as i32));
        }
        let rc = RegSetValueExW(
            hkey,
            name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr() as *const u8,
            ((value.len() - 1) * 2) as u32,
        );
        RegCloseKey(hkey);
        if rc != 0 {
            return Err(std::io::Error::from_raw_os_error(rc as i32));
        }
        Ok(())
    }
}

/// 移除开机自启
pub fn disable() {
    let key = to_wide(RUN_KEY);
    let name = to_wide(VALUE_NAME);
    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_SET_VALUE, &mut hkey) != 0 {
            return;
        }
        let _ = RegDeleteValueW(hkey, name.as_ptr());
        RegCloseKey(hkey);
    }
}
