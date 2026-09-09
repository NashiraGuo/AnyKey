// ═══════════════════════════════════════════════════════════════
// filter_driver.rs — AnyKey Filter Driver user-mode interface
//
// Communicates with the AnyKey kernel filter driver via IOCTLs
// over a device interface (GUID_DEVINTERFACE_ANYKEY_FLT).
//
// Enabled via: #[cfg(feature = "filter-driver")]
// The only backend since Interception has been removed.
//
// v0.2: Mouse support added (IOCTL_ANYKEY_WAIT_MOUSE_INPUT etc.)
// ═══════════════════════════════════════════════════════════════

use windows_sys::Win32::Storage::FileSystem::{
    FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL,
};
// CreateFileW is in the System::IO module in windows-sys 0.59
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::Foundation::{
    HANDLE, GetLastError, GENERIC_READ, GENERIC_WRITE,
};
// windows-sys 0.59 reorganized; declare CreateFileW via extern to avoid import issues
// Raw kernel32 imports for event support (windows-sys 0.59 has these but with
// import conflicts; declare our own to avoid ambiguity).
#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: u32,
        dwShareMode: u32,
        lpSecurityAttributes: *const std::ffi::c_void,
        dwCreationDisposition: u32,
        dwFlagsAndAttributes: u32,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    fn CreateEventW(
        lpEventAttributes: *const std::ffi::c_void,
        bManualReset: i32,
        bInitialState: i32,
        lpName: *const u16,
    ) -> HANDLE;
    fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: u32) -> u32;
}
use windows_sys::Win32::Foundation::CloseHandle;  // for both driver and event handles
use std::mem;
use std::collections::HashMap;

type BOOLEAN = u8;

// ── IOCTL codes (must match public.h) ──
const FILE_DEVICE_KEYBOARD: u32 = 0x0000000b;
const ANYKEY_IOCTL_INDEX: u32 = 0x900;

const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

const METHOD_BUFFERED: u32 = 0;
const FILE_READ_DATA: u32 = 1;
const FILE_WRITE_DATA: u32 = 2;

const IOCTL_ANYKEY_WAIT_INPUT: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA);
const IOCTL_ANYKEY_SEND_OUTPUT: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 1, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA);
const IOCTL_ANYKEY_GET_DEVICE_COUNT: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 2, METHOD_BUFFERED, FILE_READ_DATA);
const IOCTL_ANYKEY_GET_DEVICE_INFO: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 3, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA);
const IOCTL_ANYKEY_SET_INTERCEPT: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 4, METHOD_BUFFERED, FILE_WRITE_DATA);
const IOCTL_ANYKEY_SET_EVENT: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 5, METHOD_BUFFERED, FILE_WRITE_DATA);
const IOCTL_ANYKEY_HEARTBEAT: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 6, METHOD_BUFFERED, FILE_READ_DATA);
const IOCTL_ANYKEY_ENUM_DEVICES: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 7, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA);
const IOCTL_ANYKEY_GET_STATUS: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 8, METHOD_BUFFERED, FILE_READ_DATA);

// Mouse IOCTLs (v0.2)
const IOCTL_ANYKEY_WAIT_MOUSE_INPUT: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 9, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA);
const IOCTL_ANYKEY_SEND_MOUSE_OUTPUT: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 10, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA);
const IOCTL_ANYKEY_SET_MOUSE_MOVE: u32 =
    ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 11, METHOD_BUFFERED, FILE_WRITE_DATA);

// ── Heartbeat response struct (matches public.h) ──
#[repr(C)]
pub struct AnyKeyHeartbeatResponse {
    pub driver_version: u32,
    pub queue_depth: u32,
    pub device_count: u32,
    pub timestamp: i64,     // LARGE_INTEGER
    pub state_flags: u32,
}

pub const ANYKEY_STATE_HEALTHY: u32 = 0x00;
pub const ANYKEY_STATE_INTERCEPTING: u32 = 0x01;
pub const ANYKEY_STATE_EMERGENCY: u32 = 0x02;

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

/// Intercept request (v0.4: per-device, matches ANYKEY_INTERCEPT_REQUEST in public.h).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AnyKeyInterceptRequest {
    pub device_id: u32,   // 0 = all devices
    pub enable: u8,       // 1 = intercept, 0 = passthrough
}

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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AnyKeyDriverStatus {
    pub device_count: u32,
    pub flags: u32,
    pub intercepting_count: u32,  // v0.4: how many devices are intercepting
}
unsafe impl Send for AnyKeyDeviceInfo {}

// Device flags
pub const ANYKEY_FLAG_DEVICE_CHANGED: u32 = 0x01;
pub const ANYKEY_DEV_FLAG_PS2: u32 = 0x00000001;
pub const ANYKEY_DEV_FLAG_NO_SERIAL: u32 = 0x00000002;
pub const ANYKEY_DEV_FLAG_VIRTUAL: u32 = 0x00000004;
pub const ANYKEY_DEV_FLAG_MOUSE: u32 = 0x00000008;

// Key state flags
pub const ANYKEY_KEY_MAKE: u16 = 0x00;
pub const ANYKEY_KEY_BREAK: u16 = 0x01;
pub const ANYKEY_KEY_E0: u16 = 0x02;
pub const ANYKEY_KEY_E1: u16 = 0x04;

// ── Filter driver handle ──

pub struct FilterDriver {
    handle: HANDLE,
    h_event: HANDLE,   // valid after register_event()
}

impl FilterDriver {
    pub fn from_handle(handle: HANDLE) -> Self {
        FilterDriver { handle, h_event: std::ptr::null_mut() }
    }

    /// Open the AnyKey filter control device via symbolic link.
    /// The driver exposes \\.\AnyKeyFlt (not a PnP device interface).
    pub fn open() -> Result<Self, String> {
        // Use symbolic link directly — control device doesn't support
        // WdfDeviceCreateDeviceInterface (non-PnP), so SetupDi won't find it.
        let path_wide: Vec<u16> = "\\\\.\\AnyKeyFlt\0"
            .encode_utf16()
            .collect();

        let handle = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };

        if handle.is_null() || handle == -1isize as HANDLE {
            return Err(format!("CreateFileW(\\\\.\\AnyKeyFlt) failed: {}", unsafe { GetLastError() }));
        }

        Ok(FilterDriver { handle, h_event: std::ptr::null_mut() })
    }

    /// Enable or disable per-device input interception (v0.4).
    /// device_id=0 applies to ALL devices; device_id=N applies to device N only.
    pub fn set_device_intercept(&self, device_id: u32, enable: bool) -> Result<(), String> {
        let req = AnyKeyInterceptRequest {
            device_id,
            enable: if enable { 1 } else { 0 },
        };
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_SET_INTERCEPT,
                &req as *const _ as *const std::ffi::c_void,
                mem::size_of::<AnyKeyInterceptRequest>() as u32,
                std::ptr::null_mut(),
                0,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!("SET_INTERCEPT(dev={}, enable={}) failed: {}", device_id, enable, unsafe { GetLastError() }));
        }
        Ok(())
    }

    /// Send heartbeat to driver. Called by watchdog thread every ~5s.
    /// Returns driver status info on success.
    pub fn heartbeat(&self) -> Result<AnyKeyHeartbeatResponse, String> {
        let mut resp: AnyKeyHeartbeatResponse = unsafe { std::mem::zeroed() };
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_HEARTBEAT,
                std::ptr::null(),
                0,
                &mut resp as *mut _ as *mut std::ffi::c_void,
                mem::size_of::<AnyKeyHeartbeatResponse>() as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!("HEARTBEAT failed: {}", unsafe { GetLastError() }));
        }
        Ok(resp)
    }

    /// Poll for available input events (non-blocking).
    /// Returns a Vec of events. Empty Vec means no data available.
    pub fn poll_input(&self) -> Result<Vec<AnyKeyInputEvent>, String> {
        const MAX_EVENTS: usize = 16;
        let mut buf: [AnyKeyInputEvent; MAX_EVENTS] = unsafe { std::mem::zeroed() };
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_WAIT_INPUT,
                std::ptr::null(),
                0,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                (MAX_EVENTS * mem::size_of::<AnyKeyInputEvent>()) as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!("WAIT_INPUT failed: {}", unsafe { GetLastError() }));
        }

        let count = bytes_returned as usize / mem::size_of::<AnyKeyInputEvent>();
        Ok(buf[..count.min(MAX_EVENTS)].to_vec())
    }

    /// Poll for available mouse input events (non-blocking, v0.2).
    /// Returns a Vec of mouse events. Empty Vec means no data available.
    pub fn poll_mouse_input(&self) -> Result<Vec<AnyKeyMouseEvent>, String> {
        const MAX_EVENTS: usize = 32;  // mouse can generate more events (movement at high rate)
        let mut buf: [AnyKeyMouseEvent; MAX_EVENTS] = unsafe { std::mem::zeroed() };
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_WAIT_MOUSE_INPUT,
                std::ptr::null(),
                0,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                (MAX_EVENTS * mem::size_of::<AnyKeyMouseEvent>()) as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!("WAIT_MOUSE_INPUT failed: {}", unsafe { GetLastError() }));
        }

        let count = bytes_returned as usize / mem::size_of::<AnyKeyMouseEvent>();
        Ok(buf[..count.min(MAX_EVENTS)].to_vec())
    }

    /// Register a user-mode event that the driver signals when input arrives.
    /// After calling this, use poll_input_blocking() for zero-latency waiting.
    pub fn register_event(&mut self) -> Result<(), String> {
        // Create auto-reset event
        let h = unsafe {
            CreateEventW(
                std::ptr::null(),   // default security
                0,                  // auto-reset (not manual)
                0,                  // initial = nonsignaled
                std::ptr::null(),   // unnamed
            )
        };
        if h.is_null() || h == (-1isize as HANDLE) {
            return Err(format!("CreateEventW failed: {}", unsafe { GetLastError() }));
        }

        let mut bytes_returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_SET_EVENT,
                &h as *const _ as *const std::ffi::c_void,
                mem::size_of::<HANDLE>() as u32,
                std::ptr::null_mut(),
                0,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            unsafe { CloseHandle(h); }
            return Err(format!("SET_EVENT IOCTL failed: {}", unsafe { GetLastError() }));
        }

        self.h_event = h;
        Ok(())
    }

    /// Poll with timeout. Returns empty vec on timeout, events otherwise.
    /// Used for the main loop to wake periodically for timer processing.
    pub fn poll_input_timeout(&self, timeout_ms: u32) -> Result<Vec<AnyKeyInputEvent>, String> {
        if self.h_event.is_null() {
            return Err("register_event() not called".to_string());
        }

        const WAIT_TIMEOUT: u32 = 0x00000102;
        let result = unsafe { WaitForSingleObject(self.h_event, timeout_ms) };
        if result == WAIT_TIMEOUT {
            return Ok(Vec::new()); // timeout, no events
        }

        // Event signaled: drain
        self.poll_input()
    }

    /// Block until input is available, then drain. Zero CPU, zero latency.
    /// Requires register_event() to be called first.
    pub fn poll_input_blocking(&self) -> Result<Vec<AnyKeyInputEvent>, String> {
        if self.h_event.is_null() {
            return Err("register_event() not called".to_string());
        }

        // Wait for driver to signal the event
        const INFINITE: u32 = 0xFFFFFFFF;
        unsafe { WaitForSingleObject(self.h_event, INFINITE); }

        // Now drain (non-blocking - data is already in the queue)
        self.poll_input()
    }

    /// Wait for any input event (keyboard OR mouse), then drain both queues (v0.2).
    /// Returns (keyboard_events, mouse_events). Either Vec may be empty.
    /// This is the preferred method for the main loop: one wait, two drains.
    pub fn poll_all(&self, timeout_ms: u32) -> Result<(Vec<AnyKeyInputEvent>, Vec<AnyKeyMouseEvent>), String> {
        if self.h_event.is_null() {
            return Err("register_event() not called".to_string());
        }

        const WAIT_TIMEOUT: u32 = 0x00000102;
        let result = unsafe { WaitForSingleObject(self.h_event, timeout_ms) };
        if result == WAIT_TIMEOUT {
            return Ok((Vec::new(), Vec::new()));
        }

        // Event signaled: drain both keyboard and mouse queues
        let kb = self.poll_input().unwrap_or_default();
        let ms = self.poll_mouse_input().unwrap_or_default();
        Ok((kb, ms))
    }

    /// Inject a keyboard output event.
    /// The event is injected into kbdclass and appears as real hardware input.
    pub fn send_output(&self, event: &AnyKeyOutputEvent) -> Result<(), String> {
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_SEND_OUTPUT,
                event as *const _ as *const std::ffi::c_void,
                mem::size_of::<AnyKeyOutputEvent>() as u32,
                std::ptr::null_mut(),
                0,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!("SEND_OUTPUT failed: {}", unsafe { GetLastError() }));
        }
        Ok(())
    }

    /// Inject a mouse output event (v0.2).
    /// The event is injected into mouclass and appears as real mouse input.
    pub fn send_mouse_output(&self, event: &AnyKeyMouseOutputEvent) -> Result<(), String> {
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_SEND_MOUSE_OUTPUT,
                event as *const _ as *const std::ffi::c_void,
                mem::size_of::<AnyKeyMouseOutputEvent>() as u32,
                std::ptr::null_mut(),
                0,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!("SEND_MOUSE_OUTPUT failed: {}", unsafe { GetLastError() }));
        }
        Ok(())
    }

    /// Enable/disable mouse movement queuing for gesture tracking (v0.2).
    /// When enabled, movement events are also queued and signaled to the engine
    /// (in addition to always being forwarded to the system for cursor movement).
    pub fn set_mouse_move(&self, enable: bool) -> Result<(), String> {
        let input: BOOLEAN = if enable { 1 } else { 0 };
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_SET_MOUSE_MOVE,
                &input as *const _ as *const std::ffi::c_void,
                mem::size_of::<BOOLEAN>() as u32,
                std::ptr::null_mut(),
                0,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!("SET_MOUSE_MOVE failed: {}", unsafe { GetLastError() }));
        }
        Ok(())
    }

    /// Get the number of keyboard devices.
    pub fn get_device_count(&self) -> Result<u32, String> {
        let mut count: u32 = 0;
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_GET_DEVICE_COUNT,
                std::ptr::null(),
                0,
                &mut count as *mut _ as *mut std::ffi::c_void,
                mem::size_of::<u32>() as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!("GET_DEVICE_COUNT failed: {}", unsafe { GetLastError() }));
        }
        Ok(count)
    }

    /// Get device info for a specific device index.
    pub fn get_device_info(&self, device_id: u32) -> Result<AnyKeyDeviceInfo, String> {
        let mut info: AnyKeyDeviceInfo = unsafe { std::mem::zeroed() };
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_GET_DEVICE_INFO,
                &device_id as *const _ as *const std::ffi::c_void,
                mem::size_of::<u32>() as u32,
                &mut info as *mut _ as *mut std::ffi::c_void,
                mem::size_of::<AnyKeyDeviceInfo>() as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!("GET_DEVICE_INFO failed: {}", unsafe { GetLastError() }));
        }
        Ok(info)
    }

    /// Enumerate devices (new IOCTL_ANYKEY_ENUM_DEVICES).
    /// buf: caller-allocated array of AnyKeyDeviceInfo.
    /// Returns the number of devices written (0 = no more).
    pub fn enum_devices(
        &self,
        req: &AnyKeyEnumDevicesRequest,
        buf: &mut [AnyKeyDeviceInfo],
    ) -> Result<u32, String> {
        let max = (buf.len() as u32).min(req.max_count);
        if max == 0 { return Ok(0); }
        let mut bytes_returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_ENUM_DEVICES,
                req as *const _ as *const std::ffi::c_void,
                mem::size_of::<AnyKeyEnumDevicesRequest>() as u32,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                (max as usize * mem::size_of::<AnyKeyDeviceInfo>()) as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(format!("ENUM_DEVICES failed: {}", unsafe { GetLastError() }));
        }
        Ok(bytes_returned / mem::size_of::<AnyKeyDeviceInfo>() as u32)
    }

    /// Get driver status (IOCTL_ANYKEY_GET_STATUS).
    pub fn get_status(&self) -> Result<AnyKeyDriverStatus, String> {
        let mut st: AnyKeyDriverStatus = unsafe { std::mem::zeroed() };
        let mut bytes_returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_ANYKEY_GET_STATUS,
                std::ptr::null(), 0,
                &mut st as *mut _ as *mut std::ffi::c_void,
                mem::size_of::<AnyKeyDriverStatus>() as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(format!("GET_STATUS failed: {}", unsafe { GetLastError() }));
        }
        Ok(st)
    }

    /// Helper: parse VID/PID from hardware ID string (UTF-16).
    /// Format: "HID\VID_046D&PID_C539&..."
    pub fn parse_vid_pid(hwid: &[u16]) -> (u16, u16) {
        let s: String = hwid
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| char::from_u32(c as u32).unwrap_or('?'))
            .collect();

        let mut vid = 0u16;
        let mut pid = 0u16;

        if let Some(pos) = s.find("VID_") {
            vid = u16::from_str_radix(&s[pos + 4..pos + 8], 16).unwrap_or(0);
        }
        if let Some(pos) = s.find("PID_") {
            pid = u16::from_str_radix(&s[pos + 4..pos + 8], 16).unwrap_or(0);
        }

        (vid, pid)
    }
}

impl Drop for FilterDriver {
    fn drop(&mut self) {
        if !self.h_event.is_null() {
            // Unregister from driver before closing event handle
            let null_handle: HANDLE = std::ptr::null_mut();
            let mut _bytes: u32 = 0;
            unsafe {
                DeviceIoControl(
                    self.handle,
                    IOCTL_ANYKEY_SET_EVENT,
                    &null_handle as *const _ as *const std::ffi::c_void,
                    mem::size_of::<HANDLE>() as u32,
                    std::ptr::null_mut(), 0,
                    &mut _bytes,
                    std::ptr::null_mut(),
                );
            }
            unsafe { CloseHandle(self.h_event); }
        }
        if !self.handle.is_null() && self.handle != -1isize as HANDLE {
            unsafe { CloseHandle(self.handle); }
        }
    }
}

