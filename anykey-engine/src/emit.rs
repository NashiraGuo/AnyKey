/// AnyKey engine - Emit module (SendInput output + scancode table)

/// Key name to scancode mapping (PS/2 Set 1)
/// 内部会先规范化别名（esc→escape、bs→backspace 等）
pub fn key_name_to_scancode(key: &str) -> Option<u16> {
    let key = crate::util::canonical_key_name(key.trim());
    let k = key.as_str();
    let map = [
        ("escape", 0x01), ("1", 0x02), ("2", 0x03), ("3", 0x04), ("4", 0x05),
        ("5", 0x06), ("6", 0x07), ("7", 0x08), ("8", 0x09), ("9", 0x0A),
        ("0", 0x0B), ("-", 0x0C), ("=", 0x0D), ("backspace", 0x0E), ("tab", 0x0F),
        ("q", 0x10), ("w", 0x11), ("e", 0x12), ("r", 0x13), ("t", 0x14),
        ("y", 0x15), ("u", 0x16), ("i", 0x17), ("o", 0x18), ("p", 0x19),
        ("[", 0x1A), ("]", 0x1B), ("enter", 0x1C), ("lctrl", 0x1D),
        ("ctrl", 0x1D), ("lcontrol", 0x1D), ("a", 0x1E),
        ("s", 0x1F), ("d", 0x20), ("f", 0x21), ("g", 0x22), ("h", 0x23),
        ("j", 0x24), ("k", 0x25), ("l", 0x26), (";", 0x27), ("'", 0x28),
        ("`", 0x29), ("lshift", 0x2A),
        // Alias: bare modifier names → left side
        ("shift", 0x2A), ("rshift", 0x36), ("\\", 0x2B), ("z", 0x2C), ("x", 0x2D),
        ("c", 0x2E), ("v", 0x2F), ("b", 0x30), ("n", 0x31), ("m", 0x32),
        (",", 0x33), (".", 0x34), ("/", 0x35), ("rshift", 0x36), ("lalt", 0x38),
        ("alt", 0x38), ("space", 0x39), ("capslock", 0x3A), ("f1", 0x3B), ("f2", 0x3C),
        ("f3", 0x3D), ("f4", 0x3E), ("f5", 0x3F), ("f6", 0x40),
        ("f7", 0x41), ("f8", 0x42), ("f9", 0x43), ("f10", 0x44),
        ("f11", 0x57), ("f12", 0x58),
        ("scrolllock", 0x46), ("numlock", 0x45), ("printscreen", 0x37),
        ("pause", 0x45), ("insert", 0x52), ("home", 0x47), ("pgup", 0x49),
        ("pageup", 0x49),
        ("delete", 0x53), ("end", 0x4F), ("pgdn", 0x51),
        ("pagedown", 0x51),
        ("up", 0x48), ("down", 0x50), ("left", 0x4B), ("right", 0x4D),
        ("apps", 0x5D),
        // Extended modifier aliases
        ("rctrl", 0x1D), ("ralt", 0x38), ("lwin", 0x5B), ("rwin", 0x5C), ("win", 0x5B),
        ("numpad0", 0x52), ("numpad1", 0x4F), ("numpad2", 0x50),
        ("numpad3", 0x51), ("numpad4", 0x4B), ("numpad5", 0x4C),
        ("numpad6", 0x4D), ("numpad7", 0x47), ("numpad8", 0x48),
        ("numpad9", 0x49), ("numpadadd", 0x4E), ("numpadsub", 0x4A),
        ("numpadmul", 0x37), ("numpaddiv", 0x35), ("numpaddot", 0x53),
        ("numpadenter", 0x1C),
        ("volume_mute", 0x20), ("volume_down", 0x2E), ("volume_up", 0x30),
        ("media_next", 0x19), ("media_prev", 0x10), ("media_stop", 0x24),
        ("media_play_pause", 0x22),
        ("browser_home", 0x32), ("browser_search", 0x5E),
        ("browser_back", 0x6A), ("browser_forward", 0x69),
        ("browser_refresh", 0x6C), ("browser_favorites", 0x6B),
        ("mouseleft", 0xFF01), ("mouseright", 0xFF02), ("mousemiddle", 0xFF04),
        ("mouseside1", 0xFF08), ("mouseside2", 0xFF10),
    ];
    for (name, sc) in map {
        if name == k { return Some(sc); }
    }
    None
}

/// 检查是否为鼠标键名（含滚轮方向）
pub fn is_mouse_key_name(name: &str) -> bool {
    matches!(name.to_lowercase().as_str(),
        "mouseleft" | "mouseright" | "mousemiddle" | "mouseside1" | "mouseside2"
        | "wheelup" | "wheeldown" | "wheelleft" | "wheelright"
    )
}

/// 鼠标键名 → FLT 按钮标志（供 I/O 层纯执行）。
/// 返回 (button_flags, is_wheel)。
pub fn mouse_name_to_flags(name: &str, is_down: bool) -> Option<(u16, bool)> {
    use crate::filter_driver::{
        MOUSE_LEFT_BUTTON_DOWN, MOUSE_LEFT_BUTTON_UP,
        MOUSE_RIGHT_BUTTON_DOWN, MOUSE_RIGHT_BUTTON_UP,
        MOUSE_MIDDLE_BUTTON_DOWN, MOUSE_MIDDLE_BUTTON_UP,
        MOUSE_BUTTON_4_DOWN, MOUSE_BUTTON_4_UP,
        MOUSE_BUTTON_5_DOWN, MOUSE_BUTTON_5_UP,
        MOUSE_WHEEL, MOUSE_HWHEEL,
    };
    let lower = name.to_lowercase();
    let wheel = matches!(lower.as_str(),
        "wheelup" | "wheeldown" | "wheelleft" | "wheelright");
    let flags = match lower.as_str() {
        "mouseleft"   => if is_down { MOUSE_LEFT_BUTTON_DOWN }   else { MOUSE_LEFT_BUTTON_UP },
        "mouseright"  => if is_down { MOUSE_RIGHT_BUTTON_DOWN }  else { MOUSE_RIGHT_BUTTON_UP },
        "mousemiddle" => if is_down { MOUSE_MIDDLE_BUTTON_DOWN } else { MOUSE_MIDDLE_BUTTON_UP },
        "mouseside1"  => if is_down { MOUSE_BUTTON_4_DOWN }      else { MOUSE_BUTTON_4_UP },
        "mouseside2"  => if is_down { MOUSE_BUTTON_5_DOWN }      else { MOUSE_BUTTON_5_UP },
        "wheelup"    => MOUSE_WHEEL,
        "wheeldown"  => MOUSE_WHEEL,
        "wheelleft"  => MOUSE_HWHEEL,
        "wheelright" => MOUSE_HWHEEL,
        _ => return None,
    };
    Some((flags, wheel))
}

// ── SendInput output via windows-sys ──

/// Send a key down event via SendInput (scancode mode).
/// `extended` = true 时附加 KEYEVENTF_EXTENDEDKEY（E0 前缀键：方向键 / End / Home /
/// Insert / Delete / 右 Ctrl|Alt / Win / 小键盘 Enter|/ / 媒体键 / 音量键 等）。
/// 注意：wScan 应填「裸」扫描码（不含 0xE0 前缀），E0 用 flag 表达，否则键位错位。
pub fn send_key_down_sendinput(scancode: u16, extended: bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let mut flags = KEYEVENTF_SCANCODE;
    if extended { flags |= KEYEVENTF_EXTENDEDKEY; }
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_KEYBOARD;
    unsafe {
        input.Anonymous.ki = KEYBDINPUT {
            wVk: 0,
            wScan: scancode,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        SendInput(1, &mut input, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Send a key up event via SendInput (scancode mode). 见 `send_key_down_sendinput`。
pub fn send_key_up_sendinput(scancode: u16, extended: bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let mut flags = KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP;
    if extended { flags |= KEYEVENTF_EXTENDEDKEY; }
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_KEYBOARD;
    unsafe {
        input.Anonymous.ki = KEYBDINPUT {
            wVk: 0,
            wScan: scancode,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        SendInput(1, &mut input, std::mem::size_of::<INPUT>() as i32);
    }
}
