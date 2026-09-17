//! events.rs — 两个后端共用的 I/O 词汇。
//!
//! 这里放的是**输入/输出事件本身**（含与 public.h 逐字段对齐的 #[repr(C)] 负载）以及
//! 它们之间的纯逻辑（鼠标合并包展开）。这些东西**不属于任何一个后端**：
//!
//! - 驱动后端：把 IOCTL 收到的字节按这些结构体解释、把要发的按这些结构体回填；
//! - 免驱动后端：把 `KBDLLHOOKSTRUCT` / `MSLLHOOKSTRUCT` 投影成同一批结构体，
//!   输出再交给 `sendinput_out.rs`。
//!
//! **驱动协议**（控制设备句柄、IOCTL 码、心跳/拦截/状态/枚举请求等只对驱动有意义的
//! 结构）留在 `filter_driver.rs`。判据很简单：这种东西在免驱动后端下有对应物吗？
//! 没有 → 留在 filter_driver；有（哪怕是桩）→ 放这里。
//!
//! 本模块**不带任何 cfg** —— 它是共享词汇，不该随特性出现或消失。

use std::collections::HashMap;

// ═══ 键盘 I/O 事件 ═══

// ── C struct analogs (must match public.h layout) ──
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AnyKeyInputEvent {
    pub make_code: u16,
    pub flags: u16,
    pub device_id: u32,
    pub extra_info: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AnyKeyOutputEvent {
    pub make_code: u16,
    pub flags: u16,
    pub device_id: u32,
}

// ═══ 鼠标 I/O 事件与标志 ═══

// ── Mouse event structs (v0.2 — must match public.h) ──

/// Mouse input event from driver (via IOCTL_ANYKEY_WAIT_MOUSE_INPUT).
/// Layout matches ANYKEY_MOUSE_EVENT in public.h (24 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AnyKeyMouseEvent {
    pub device_id: u32,
    pub flags: u16,          // MOUSE_MOVE_RELATIVE(0) / MOUSE_MOVE_ABSOLUTE(1)
    pub button_flags: u16,   // combination of MOUSE_*_BUTTON_DOWN/UP, MOUSE_WHEEL, etc.
    pub button_data: i16,    // Wheel delta: +120 forward, -120 backward
    pub last_x: i32,         // Relative movement X, or absolute coordinate X
    pub last_y: i32,         // Relative movement Y, or absolute coordinate Y
    pub extra_info: u32,     // Original MOUSE_INPUT_DATA.ExtraInformation
}

/// Mouse output event to driver (via IOCTL_ANYKEY_SEND_MOUSE_OUTPUT).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AnyKeyMouseOutputEvent {
    pub device_id: u32,
    pub flags: u16,          // MOUSE_MOVE_RELATIVE (typically)
    pub button_flags: u16,
    pub button_data: i16,
    pub last_x: i32,
    pub last_y: i32,
}

// Mouse button flags (from ntddmou.h MOUSE_INPUT_DATA.ButtonFlags)
pub const MOUSE_LEFT_BUTTON_DOWN: u16   = 0x0001;
pub const MOUSE_LEFT_BUTTON_UP: u16     = 0x0002;
pub const MOUSE_RIGHT_BUTTON_DOWN: u16  = 0x0004;
pub const MOUSE_RIGHT_BUTTON_UP: u16    = 0x0008;
pub const MOUSE_MIDDLE_BUTTON_DOWN: u16 = 0x0010;
pub const MOUSE_MIDDLE_BUTTON_UP: u16   = 0x0020;
pub const MOUSE_BUTTON_4_DOWN: u16      = 0x0040;
pub const MOUSE_BUTTON_4_UP: u16        = 0x0080;
pub const MOUSE_BUTTON_5_DOWN: u16      = 0x0100;
pub const MOUSE_BUTTON_5_UP: u16        = 0x0200;
pub const MOUSE_WHEEL: u16              = 0x0400;
pub const MOUSE_HWHEEL: u16             = 0x0800;

// Mouse movement flags (from ntddmou.h MOUSE_INPUT_DATA.Flags)
pub const MOUSE_MOVE_RELATIVE: u16    = 0x00;
pub const MOUSE_MOVE_ABSOLUTE: u16    = 0x01;
pub const MOUSE_VIRTUAL_DESKTOP: u16  = 0x02;
pub const MOUSE_ATTRIBUTES_CHANGED: u16 = 0x04;

// ═══ MouseEventTranslator ═══

// ── MouseEventTranslator (v0.4.x) ──
// Splits merged mouse packets (multiple button edges in one ButtonFlags mask,
// e.g. LEFT_DOWN|RIGHT_UP = 0x0009 when holding left and releasing right) into
// separate single-edge events, and drops self-contradictory edges by tracking
// each device's pressed mask. This is the engine-side counterpart to the
// driver's raw passthrough: the pipeline only understands single-bit edges,
// so merged packets must be expanded here before reaching key_down/key_up.

/// Translates raw driver mouse packets into clean single-edge events.
///
/// Windows `MOUSE_INPUT_DATA.ButtonFlags` is a bitmask, so one packet may carry
/// several edges at once. This translator:
///   - emits EVERY valid edge, in fixed order L→R→M→X1→X2 (down before up),
///   - drops self-contradictory edges (a DOWN for an already-pressed button,
///     an UP for a button never seen down) by tracking per-device state,
///   - passes wheel flags through unchanged (direction encoded in the name).
#[derive(Default)]
pub struct MouseEventTranslator {
    /// device_id -> pressed down-bit mask (bit0=L, bit2=R, bit4=M, bit6=X1, bit8=X2)
    pressed: HashMap<u32, u16>,
}

impl MouseEventTranslator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset all per-device state (call when a session starts/ends).
    pub fn reset(&mut self) {
        self.pressed.clear();
    }

    /// Drop state for one device (call when it unplugs).
    pub fn reset_device(&mut self, device_id: u32) {
        self.pressed.remove(&device_id);
    }

    /// Translate one raw packet into clean single-edge events.
    /// Returns one entry per VALID edge; `(name, is_down)`.
    pub fn translate(&mut self, evt: &AnyKeyMouseEvent) -> Vec<(&'static str, bool)> {
        let mut out = Vec::with_capacity(4);
        let st = self.pressed.entry(evt.device_id).or_insert(0u16);

        const BTNS: [(u16, &str); 5] = [
            (MOUSE_LEFT_BUTTON_DOWN, "MouseLeft"),
            (MOUSE_RIGHT_BUTTON_DOWN, "MouseRight"),
            (MOUSE_MIDDLE_BUTTON_DOWN, "MouseMiddle"),
            (MOUSE_BUTTON_4_DOWN, "MouseSide1"),
            (MOUSE_BUTTON_5_DOWN, "MouseSide2"),
        ];
        for (down_bit, name) in BTNS {
            let up_bit = down_bit << 1;
            if evt.button_flags & down_bit != 0 {
                if *st & down_bit == 0 {
                    *st |= down_bit;
                    out.push((name, true));
                }
                // else: already pressed -> spurious re-assertion, dropped
            }
            if evt.button_flags & up_bit != 0 {
                if *st & down_bit != 0 {
                    *st &= !down_bit;
                    out.push((name, false));
                }
                // else: never pressed -> spurious up, dropped
            }
        }

        // Wheel: self-contained event, direction encoded in the name.
        if evt.button_flags & (MOUSE_WHEEL | MOUSE_HWHEEL) != 0 {
            let name = if evt.button_flags & MOUSE_WHEEL != 0 {
                if evt.button_data > 0 { "WheelUp" } else { "WheelDown" }
            } else if evt.button_data > 0 {
                "WheelRight"
            } else {
                "WheelLeft"
            };
            out.push((name, true));
        }
        out
    }
}

// ═══ 设备清单负载 ═══

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AnyKeyDeviceInfo {
    pub device_id: u32,
    pub hardware_id: [u16; 128],
    pub container_id: [u16; 40],
    pub friendly_name: [u16; 64],
    pub vendor_id: u16,
    pub product_id: u16,
    pub is_keyboard: u8,
    pub is_mouse: u8,
    pub has_serial_nr: u8,   // BOOLEAN
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AnyKeyEnumDevicesRequest {
    pub index: u32,
    pub max_count: u32,
}

// ═══ Send 标记 ═══

unsafe impl Send for AnyKeyDeviceInfo {}

// ═══ 设备标志（DescribeDevice 回填用；驱动状态位见 filter_driver.rs）═══

pub const ANYKEY_DEV_FLAG_PS2: u32 = 0x00000001;
pub const ANYKEY_DEV_FLAG_NO_SERIAL: u32 = 0x00000002;
pub const ANYKEY_DEV_FLAG_VIRTUAL: u32 = 0x00000004;
pub const ANYKEY_DEV_FLAG_MOUSE: u32 = 0x00000008;

// ═══ 键状态标志 ═══

// Key state flags
pub const ANYKEY_KEY_MAKE: u16 = 0x00;
pub const ANYKEY_KEY_BREAK: u16 = 0x01;
pub const ANYKEY_KEY_E0: u16 = 0x02;
pub const ANYKEY_KEY_E1: u16 = 0x04;
