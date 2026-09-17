//! sendinput_out.rs — 免驱动后端的输出侧：鼠标按键 / 滚轮 / 移动走 `SendInput`。
//!
//! 键盘输出**不在这里** —— 直接复用 `emit.rs::send_key_down_sendinput/up_sendinput`
//! （已是扫描码模式 + extended 标志，见 `hook_input.rs::send_output`）。
//!
//! # 为什么按钮标志要过一张表
//!
//! 引擎内部沿用驱动约定（ntddmou 的 `MOUSE_INPUT_DATA.ButtonFlags`），而 `SendInput` 用的是
//! `MOUSEEVENTF_*` —— 两套位值**整体错开一位**（左键按下：驱动 `0x0001` / SendInput `0x0002`），
//! 所以不能透传。4/5 号键更特殊：要发 `XDOWN/XUP` 并把 `mouseData` 设为 `XBUTTON1/2`，
//! 而驱动侧只在 `ButtonFlags` 里给一个位。
//!
//! # 一次 SendInput 提交多个 INPUT
//!
//! 引擎侧是"逐条提交"（`drain_emit_log_flt` 一次只发一个事件），但一个事件可能同时带
//! 多个边沿（例如鼠标包被合并）。这里把该事件展开成多个 `INPUT` **一次提交**，
//! 顺序与展开顺序一致，避免中间被别的输入插队。

use crate::filter_driver::{
    AnyKeyMouseOutputEvent, MOUSE_BUTTON_4_DOWN, MOUSE_BUTTON_4_UP, MOUSE_BUTTON_5_DOWN,
    MOUSE_BUTTON_5_UP, MOUSE_HWHEEL, MOUSE_LEFT_BUTTON_DOWN, MOUSE_LEFT_BUTTON_UP,
    MOUSE_MIDDLE_BUTTON_DOWN, MOUSE_MIDDLE_BUTTON_UP, MOUSE_MOVE_ABSOLUTE, MOUSE_RIGHT_BUTTON_DOWN,
    MOUSE_RIGHT_BUTTON_UP, MOUSE_VIRTUAL_DESKTOP, MOUSE_WHEEL,
};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// windows-sys 0.59 未导出这两个常量。
const XBUTTON1: u32 = 0x0001;
const XBUTTON2: u32 = 0x0002;

fn mouse_input(dx: i32, dy: i32, mouse_data: u32, flags: u32) -> INPUT {
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_MOUSE;
    input.Anonymous.mi = windows_sys::Win32::UI::Input::KeyboardAndMouse::MOUSEINPUT {
        dx,
        dy,
        mouseData: mouse_data,
        dwFlags: flags,
        time: 0,
        dwExtraInfo: 0,
    };
    input
}

/// 注入一个鼠标输出事件。返回 Err 表示 `SendInput` 没有完整插入（UIPI / 安全软件）。
pub fn send_mouse_output(ev: &AnyKeyMouseOutputEvent) -> Result<(), String> {
    let bf = ev.button_flags;
    let mut inputs: Vec<INPUT> = Vec::with_capacity(4);

    // ── 滚轮（自含事件：方向在 button_data 的符号里） ──
    if bf & MOUSE_WHEEL != 0 {
        // i16 → i32 → u32：保留符号位（SendInput 按有符号解释 mouseData）
        inputs.push(mouse_input(0, 0, ev.button_data as i32 as u32, MOUSEEVENTF_WHEEL));
    }
    if bf & MOUSE_HWHEEL != 0 {
        inputs.push(mouse_input(0, 0, ev.button_data as i32 as u32, MOUSEEVENTF_HWHEEL));
    }

    // ── 三键：驱动位 → MOUSEEVENTF（整体错开一位） ──
    if bf & MOUSE_LEFT_BUTTON_DOWN != 0 {
        inputs.push(mouse_input(0, 0, 0, MOUSEEVENTF_LEFTDOWN));
    }
    if bf & MOUSE_LEFT_BUTTON_UP != 0 {
        inputs.push(mouse_input(0, 0, 0, MOUSEEVENTF_LEFTUP));
    }
    if bf & MOUSE_RIGHT_BUTTON_DOWN != 0 {
        inputs.push(mouse_input(0, 0, 0, MOUSEEVENTF_RIGHTDOWN));
    }
    if bf & MOUSE_RIGHT_BUTTON_UP != 0 {
        inputs.push(mouse_input(0, 0, 0, MOUSEEVENTF_RIGHTUP));
    }
    if bf & MOUSE_MIDDLE_BUTTON_DOWN != 0 {
        inputs.push(mouse_input(0, 0, 0, MOUSEEVENTF_MIDDLEDOWN));
    }
    if bf & MOUSE_MIDDLE_BUTTON_UP != 0 {
        inputs.push(mouse_input(0, 0, 0, MOUSEEVENTF_MIDDLEUP));
    }

    // ── 4/5 号键：XDOWN/XUP + mouseData = XBUTTON1/2 ──
    if bf & MOUSE_BUTTON_4_DOWN != 0 {
        inputs.push(mouse_input(0, 0, XBUTTON1, MOUSEEVENTF_XDOWN));
    }
    if bf & MOUSE_BUTTON_4_UP != 0 {
        inputs.push(mouse_input(0, 0, XBUTTON1, MOUSEEVENTF_XUP));
    }
    if bf & MOUSE_BUTTON_5_DOWN != 0 {
        inputs.push(mouse_input(0, 0, XBUTTON2, MOUSEEVENTF_XDOWN));
    }
    if bf & MOUSE_BUTTON_5_UP != 0 {
        inputs.push(mouse_input(0, 0, XBUTTON2, MOUSEEVENTF_XUP));
    }

    // ── 移动（last_x/last_y 均为 0 时跳过，避免把"纯按钮事件"变成一次 0 位移移动） ──
    if ev.last_x != 0 || ev.last_y != 0 {
        let absolute = (ev.flags & MOUSE_MOVE_ABSOLUTE) != 0;
        if absolute {
            // 驱动的绝对坐标是像素；SendInput 要求归一化到 0..65535。
            // 带 MOUSE_VIRTUAL_DESKTOP 时坐标是虚拟桌面空间（跨屏），否则是主屏空间。
            let virtual_desk = (ev.flags & MOUSE_VIRTUAL_DESKTOP) != 0;
            let (nx, ny) = normalize_absolute(ev.last_x, ev.last_y, virtual_desk);
            let mut flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE;
            if virtual_desk {
                flags |= MOUSEEVENTF_VIRTUALDESK;
            }
            inputs.push(mouse_input(nx, ny, 0, flags));
        } else {
            inputs.push(mouse_input(ev.last_x, ev.last_y, 0, MOUSEEVENTF_MOVE));
        }
    }

    if inputs.is_empty() {
        return Ok(());
    }
    let want = inputs.len() as u32;
    let got = unsafe {
        SendInput(want, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32)
    };
    if got != want {
        return Err(format!(
            "SendInput inserted {}/{} mouse input(s) — UIPI or security software",
            got, want
        ));
    }
    Ok(())
}

/// 像素坐标 → `SendInput` 绝对坐标（0..65535）。`virtual_desk` 决定用哪套屏幕度量。
fn normalize_absolute(x: i32, y: i32, virtual_desk: bool) -> (i32, i32) {
    let (ox, oy, w, h) = unsafe {
        if virtual_desk {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        } else {
            (
                0,
                0,
                GetSystemMetrics(SM_CXSCREEN),
                GetSystemMetrics(SM_CYSCREEN),
            )
        }
    };
    let norm = |v: i32, origin: i32, span: i32| -> i32 {
        let span = if span > 1 { span - 1 } else { 1 };
        (((v - origin) as i64 * 65535 / span as i64).clamp(0, 65535)) as i32
    };
    (norm(x, ox, w), norm(y, oy, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_spans_full_range() {
        // 单屏 1920x1080：左上 → 0，右下 → 65535
        let (x0, y0) = normalize_absolute(0, 0, false);
        assert_eq!((x0, y0), (0, 0));
        unsafe {
            let w = GetSystemMetrics(SM_CXSCREEN);
            let h = GetSystemMetrics(SM_CYSCREEN);
            let (x1, y1) = normalize_absolute(w - 1, h - 1, false);
            assert_eq!((x1, y1), (65535, 65535));
        }
    }

    #[test]
    fn normalize_clamps_out_of_range() {
        let (x, y) = normalize_absolute(-10_000, 999_999, false);
        assert_eq!((x, y), (0, 65535));
    }
}
