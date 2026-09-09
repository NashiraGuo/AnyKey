"""AnyKey Filter Driver - Python wrapper via DeviceIoControl.

Opens \\.\\AnyKeyFlt and uses IOCTL to enumerate devices and query status.
Falls back gracefully if the driver is not installed.
"""
import ctypes
import ctypes.wintypes as w
import struct

# ── IOCTL codes ──
# ANYKEY_IOCTL_INDEX 必须与驱动 sys/public.h 中的定义保持一致（当前为 0x900）。
# 若驱动升级该基数，此处必须同步修改，否则所有 DeviceIoControl 都会被驱动拒绝。
FILE_DEVICE_KEYBOARD = 0x0B
METHOD_BUFFERED = 0
ANYKEY_IOCTL_INDEX = 0x900

def _ctl_code(dev, func, method, access):
    return (dev << 16) | (access << 14) | (func << 2) | method

IOCTL_ENUM_DEVICES = _ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 7,
                                METHOD_BUFFERED, 3)  # FILE_READ | FILE_WRITE
IOCTL_GET_STATUS    = _ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 8,
                                METHOD_BUFFERED, 1)  # FILE_READ
IOCTL_SET_INTERCEPT = _ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 4,
                                METHOD_BUFFERED, 2)  # FILE_WRITE
IOCTL_WAIT_INPUT    = _ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 0,
                                METHOD_BUFFERED, 3)  # FILE_READ | FILE_WRITE
IOCTL_SET_CAPTURE   = _ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 12,
                                METHOD_BUFFERED, 2)  # FILE_WRITE
IOCTL_WAIT_MOUSE_INPUT = _ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 9,
                                   METHOD_BUFFERED, 3)  # FILE_READ | FILE_WRITE
IOCTL_SEND_MOUSE_OUTPUT = _ctl_code(FILE_DEVICE_KEYBOARD, ANYKEY_IOCTL_INDEX + 10,
                                    METHOD_BUFFERED, 3)

# ── AnyKeyDeviceInfo (mirrors ANYKEY_DEVICE_INFO in sys/public.h) ──
# Must stay byte-identical with the driver's C struct and the engine's Rust
# repr(C) struct. Using a ctypes.Structure (native MSVC x64 alignment) lets
# Python compute the real size instead of a hard-coded value, so the per-device
# stride can never silently drift again. Previous hard-coded 478 missed the
# 1-byte padding before the 4-byte-aligned `flags`, shifting every device by
# 2 bytes and corrupting container_id / friendly_name with NULs + neighbor bytes.
class _AnyKeyDeviceInfo(ctypes.Structure):
    _fields_ = [
        ("device_id", ctypes.c_uint32),           # ULONG DeviceId
        ("hardware_id", ctypes.c_uint16 * 128),   # WCHAR HardwareId[128]
        ("container_id", ctypes.c_uint16 * 40),   # WCHAR ContainerId[40]
        ("friendly_name", ctypes.c_uint16 * 64),  # WCHAR FriendlyName[64]
        ("vendor_id", ctypes.c_uint16),           # USHORT VendorId
        ("product_id", ctypes.c_uint16),          # USHORT ProductId
        ("is_keyboard", ctypes.c_uint8),          # BOOLEAN IsKeyboard
        ("is_mouse", ctypes.c_uint8),             # BOOLEAN IsMouse
        ("has_serial_nr", ctypes.c_uint8),        # BOOLEAN HasSerialNumber
        ("flags", ctypes.c_uint32),               # ULONG Flags (4-byte aligned)
    ]


def _wchar_arr_to_str(arr):
    """Decode a fixed WCHAR array (u16) to str, stopping at the first NUL."""
    chars = []
    for v in arr:
        if v == 0:
            break
        chars.append(chr(v))
    return "".join(chars)


DEVICE_INFO_SIZE = ctypes.sizeof(_AnyKeyDeviceInfo)  # 480 on MSVC x64 (1-byte pad before flags)
MAX_DEVICES = 16

# ── Windows API ──
kernel32 = ctypes.windll.kernel32
CreateFileW = kernel32.CreateFileW
DeviceIoControl = kernel32.DeviceIoControl
CloseHandle = kernel32.CloseHandle

GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
OPEN_EXISTING = 3
FILE_ATTRIBUTE_NORMAL = 0x80
INVALID_HANDLE_VALUE = w.HANDLE(-1).value

# ── OVERLAPPED (x64 layout) ──
class _OVERLAPPED(ctypes.Structure):
    _fields_ = [
        ("Internal", ctypes.c_void_p),
        ("InternalHigh", ctypes.c_void_p),
        ("Offset", ctypes.c_ulonglong),   # union Offset/OffsetHigh/Pointer
        ("hEvent", ctypes.c_void_p),      # event handle lives at offset 24 on x64
    ]

kernel32.CreateEventW.restype = w.HANDLE
kernel32.CreateEventW.argtypes = [ctypes.c_void_p, w.BOOL, w.BOOL, w.LPCWSTR]
kernel32.GetOverlappedResult.restype = w.BOOL
kernel32.GetOverlappedResult.argtypes = [w.HANDLE, ctypes.c_void_p, ctypes.POINTER(w.DWORD), w.BOOL]
kernel32.WaitForSingleObject.restype = w.DWORD
kernel32.WaitForSingleObject.argtypes = [w.HANDLE, w.DWORD]
kernel32.CancelIo.restype = w.BOOL
kernel32.CancelIo.argtypes = [w.HANDLE]
kernel32.CloseHandle.restype = w.BOOL
kernel32.CloseHandle.argtypes = [w.HANDLE]


class FilterDriver:
    def __init__(self):
        self._handle = None

    def open(self):
        if self._handle:
            return True
        h = CreateFileW(
            r"\\.\AnyKeyFlt",
            GENERIC_READ | GENERIC_WRITE,
            0, None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, None)
        if h == INVALID_HANDLE_VALUE:
            return False
        self._handle = h
        return True

    def close(self):
        if self._handle:
            CloseHandle(self._handle)
            self._handle = None

    def is_open(self):
        return self._handle is not None

    def enum_devices(self):
        """Return a list of dicts with device info."""
        if not self.open():
            return []
        devices = []
        index = 0
        req = struct.pack("<II", index, MAX_DEVICES)
        buf = (ctypes.c_ubyte * (DEVICE_INFO_SIZE * MAX_DEVICES))()
        while True:
            req = struct.pack("<II", index, MAX_DEVICES)
            ret = w.DWORD()
            ok = DeviceIoControl(
                self._handle, IOCTL_ENUM_DEVICES,
                ctypes.byref(ctypes.c_buffer(req, 8)),
                8,
                ctypes.byref(buf),
                ctypes.sizeof(buf),
                ctypes.byref(ret), None)
            if not ok:
                break
            count = ret.value // DEVICE_INFO_SIZE
            if count == 0:
                break
            for i in range(count):
                off = i * DEVICE_INFO_SIZE
                raw = bytes(buf[off:off + DEVICE_INFO_SIZE])
                dev = self._parse_device_info(raw)
                devices.append(dev)
            index += count
        return devices

    def _parse_device_info(self, raw):
        """Parse a single ANYKEY_DEVICE_INFO from binary (raw is DEVICE_INFO_SIZE bytes)."""
        info = _AnyKeyDeviceInfo.from_buffer_copy(raw)
        return {
            "device_id": info.device_id,
            "hardware_id": _wchar_arr_to_str(info.hardware_id),
            "container_id": _wchar_arr_to_str(info.container_id),
            "friendly_name": _wchar_arr_to_str(info.friendly_name),
            "vendor_id": info.vendor_id,
            "product_id": info.product_id,
            "is_keyboard": info.is_keyboard != 0,
            "is_mouse": info.is_mouse != 0,
            "has_serial": info.has_serial_nr != 0,
            "flags": info.flags,
        }

    def get_status(self):
        """Return (device_count, flags, intercept_enabled) or None."""
        if not self.open():
            return None
        buf = (ctypes.c_ubyte * 12)()
        ret = w.DWORD()
        ok = DeviceIoControl(
            self._handle, IOCTL_GET_STATUS,
            None, 0,
            ctypes.byref(buf), ctypes.sizeof(buf),
            ctypes.byref(ret), None)
        if not ok:
            return None
        dc = struct.unpack_from("<I", buf, 0)[0]
        flags = struct.unpack_from("<I", buf, 4)[0]
        intercept = buf[8] != 0
        return (dc, flags, intercept)

    def set_capture(self, enabled):
        """Enable/disable passthrough capture mode (passive device identification).

        When enabled, the driver mirrors keyboard/mouse input into its queues
        WITHOUT consuming it, so a caller can poll_input()/poll_mouse_input()
        while keystrokes and mouse events still reach the OS. This is what lets
        the GUI "识别" feature work without intercept mode (which would freeze
        the real keyboard). Returns True on success.
        """
        if not self.open():
            return False
        in_buf = (ctypes.c_ubyte * 1)(1 if enabled else 0)
        ret = w.DWORD()
        ok = DeviceIoControl(
            self._handle, IOCTL_SET_CAPTURE,
            ctypes.byref(in_buf), 1,
            None, 0,
            ctypes.byref(ret), None)
        return ok

    def _wait_overlapped(self, buf, overlapped, timeout_ms):
        """Wait for a pending WAIT_INPUT/WAIT_MOUSE_INPUT IOCTL.

        Returns the number of bytes transferred, or 0 on timeout/error.
        Uses a proper event handle (offset 24 on x64) so the wait actually
        fires on completion. On timeout the pending IRP is cancelled so it
        cannot accumulate and deadlock a later call on the same handle.
        """
        if kernel32.WaitForSingleObject(overlapped.hEvent, timeout_ms) != 0:
            kernel32.CancelIo(self._handle)  # abandon the pending wait
            return 0
        res = w.DWORD()
        if kernel32.GetOverlappedResult(self._handle, ctypes.byref(overlapped),
                                        ctypes.byref(res), False):
            return res.value
        return 0

    def poll_input(self, timeout_ms=200):
        """Wait for key events. Returns list of {make_code, flags, device_id} or [] on timeout."""
        if not self.is_open():
            return []
        overlapped = _OVERLAPPED()
        overlapped.hEvent = kernel32.CreateEventW(None, True, False, None)
        if not overlapped.hEvent:
            return []
        try:
            buf = (ctypes.c_ubyte * (12 * 16))()  # up to 16 events
            ret = w.DWORD()
            ok = DeviceIoControl(
                self._handle, IOCTL_WAIT_INPUT,
                None, 0,
                ctypes.byref(buf), ctypes.sizeof(buf),
                ctypes.byref(ret), ctypes.byref(overlapped))
            if ok:
                # Immediate completion (event already buffered in the driver)
                return self._parse_input_events(buf, ret.value)
            if ctypes.get_last_error() != 997:  # not ERROR_IO_PENDING
                return []
            n = self._wait_overlapped(buf, overlapped, timeout_ms)
            if n:
                return self._parse_input_events(buf, n)
            return []
        finally:
            if overlapped.hEvent:
                kernel32.CloseHandle(overlapped.hEvent)

    def _parse_input_events(self, buf, byte_count):
        """Parse AnyKeyInputEvent array."""
        events = []
        for i in range(byte_count // 12):
            off = i * 12
            make_code = struct.unpack_from("<H", buf, off)[0]
            flags = struct.unpack_from("<H", buf, off + 2)[0]
            device_id = struct.unpack_from("<I", buf, off + 4)[0]
            key_name = _scancode_name(make_code, flags)
            events.append({
                "make_code": make_code,
                "flags": flags,
                "device_id": device_id,
                "key_name": key_name,
            })
        return events
    def poll_mouse_input(self, timeout_ms=200):
        """Wait for mouse events. Returns list of {device_id, flags, button_flags, button_data, last_x, last_y}."""
        if not self.is_open():
            return []
        overlapped = _OVERLAPPED()
        overlapped.hEvent = kernel32.CreateEventW(None, True, False, None)
        if not overlapped.hEvent:
            return []
        try:
            buf = (ctypes.c_ubyte * (24 * 16))()
            ret = w.DWORD()
            ok = DeviceIoControl(
                self._handle, IOCTL_WAIT_MOUSE_INPUT,
                None, 0,
                ctypes.byref(buf), ctypes.sizeof(buf),
                ctypes.byref(ret), ctypes.byref(overlapped))
            if ok:
                return self._parse_mouse_events(buf, ret.value)
            if ctypes.get_last_error() != 997:
                return []
            n = self._wait_overlapped(buf, overlapped, timeout_ms)
            if n:
                return self._parse_mouse_events(buf, n)
            return []
        finally:
            if overlapped.hEvent:
                kernel32.CloseHandle(overlapped.hEvent)

    def _parse_mouse_events(self, buf, byte_count):
        # ANYKEY_MOUSE_EVENT is 24 bytes with 2-byte padding before LastX
        # (DeviceId@0, Flags@4, ButtonFlags@6, ButtonData@8, [pad@10], LastX@12, LastY@16, ExtraInfo@20)
        events = []
        for i in range(byte_count // 24):
            off = i * 24
            device_id = struct.unpack_from("<I", buf, off)[0]
            flags = struct.unpack_from("<H", buf, off + 4)[0]
            button_flags = struct.unpack_from("<H", buf, off + 6)[0]
            button_data = struct.unpack_from("<h", buf, off + 8)[0]
            last_x = struct.unpack_from("<i", buf, off + 12)[0]
            last_y = struct.unpack_from("<i", buf, off + 16)[0]
            events.append({
                "device_id": device_id,
                "flags": flags,
                "button_flags": button_flags,
                "button_data": button_data,
                "last_x": last_x,
                "last_y": last_y,
            })
        return events



# ── Module-level convenience ──

_fd_instance = None

def get_driver():
    global _fd_instance
    if _fd_instance is None:
        _fd_instance = FilterDriver()
    return _fd_instance


# ── PS/2 Set 1 scan code → key name ──
_SCANCODE_TABLE = {
    0x01: "Esc",  0x02: "1",  0x03: "2",  0x04: "3",  0x05: "4",
    0x06: "5",    0x07: "6",  0x08: "7",  0x09: "8",  0x0A: "9",
    0x0B: "0",    0x0C: "-",  0x0D: "=",  0x0E: "BS",
    0x0F: "Tab",  0x10: "Q",  0x11: "W",  0x12: "E",  0x13: "R",
    0x14: "T",    0x15: "Y",  0x16: "U",  0x17: "I",  0x18: "O",
    0x19: "P",    0x1A: "[",  0x1B: "]",  0x1C: "Enter",
    0x1D: "LCtrl",0x1E: "A",  0x1F: "S",  0x20: "D",  0x21: "F",
    0x22: "G",    0x23: "H",  0x24: "J",  0x25: "K",  0x26: "L",
    0x27: ";",    0x28: "'",  0x29: "`",
    0x2A: "LShift",0x2B: "\\",0x2C: "Z",  0x2D: "X",  0x2E: "C",
    0x2F: "V",    0x30: "B",  0x31: "N",  0x32: "M",  0x33: ",",
    0x34: ".",    0x35: "/",  0x36: "RShift",
    0x37: "PrtSc",0x38: "LAlt",0x39: "Space",0x3A: "Caps",
    0x3B: "F1",   0x3C: "F2", 0x3D: "F3",  0x3E: "F4",
    0x3F: "F5",   0x40: "F6", 0x41: "F7",  0x42: "F8",
    0x43: "F9",   0x44: "F10",0x45: "NumLk",0x46: "ScrLk",
    0x47: "KP7",  0x48: "KP8",0x49: "KP9", 0x4A: "KP-",
    0x4B: "KP4",  0x4C: "KP5",0x4D: "KP6", 0x4E: "KP+",
    0x4F: "KP1",  0x50: "KP2",0x51: "KP3", 0x52: "KP0",
    0x53: "KP.",  0x57: "F11", 0x58: "F12",
    0x5B: "LWin", 0x5C: "RWin",0x5D: "Menu",
    # E0-extended codes
    0xE01C: "KPEnt", 0xE05B: "LWin", 0xE05C: "RWin",
    0xE048: "Up",    0xE050: "Down", 0xE04B: "Left", 0xE04D: "Right",
    0xE035: "KP/",   0xE047: "Home", 0xE04F: "End",
    0xE049: "PgUp",  0xE051: "PgDn", 0xE052: "Ins",  0xE053: "Del",
    0xE01D: "RCtrl", 0xE038: "RAlt",
}
E0_MASK = 0xE000

def _scancode_name(make_code, flags):
    """Return human-readable key name from PS/2 Set 1 scan code and flags."""
    if flags & E0_MASK:
        e0_key = 0xE000 | (make_code & 0xFF)
        if e0_key in _SCANCODE_TABLE:
            return _SCANCODE_TABLE[e0_key]
    return _SCANCODE_TABLE.get(make_code & 0xFF, f"0x{make_code:02X}")
