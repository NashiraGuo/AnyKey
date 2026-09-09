"""
Windows HID 设备枚举 + 设备识别
- scan_devices() → Raw Input 枚举键盘/鼠标
- DeviceIdentifier → Raw Input 识别键盘 + Interception DLL 识别鼠标
"""

import ctypes
from ctypes import wintypes
import os
import re
import struct
import threading

# ── 常量 ──────────────────────────────────────
RIM_TYPEKEYBOARD = 1
RIM_TYPEMOUSE = 2
RIDI_DEVICENAME = 0x20000007

# ── 结构体 ────────────────────────────────────

class RAWINPUTDEVICELIST(ctypes.Structure):
    _fields_ = [
        ("hDevice", wintypes.HANDLE),
        ("dwType", wintypes.DWORD),
    ]

class RID_DEVICE_INFO_KEYBOARD(ctypes.Structure):
    _fields_ = [
        ("dwType", wintypes.DWORD),
        ("dwKeyboardType", wintypes.DWORD),
        ("dwKeyboardSubType", wintypes.DWORD),
        ("dwKeyboardMode", wintypes.DWORD),
        ("dwNumberOfFunctionKeys", wintypes.DWORD),
        ("dwNumberOfIndicators", wintypes.DWORD),
        ("dwNumberOfKeysTotal", wintypes.DWORD),
    ]

class RID_DEVICE_INFO_MOUSE(ctypes.Structure):
    _fields_ = [
        ("dwId", wintypes.DWORD),
        ("dwNumberOfButtons", wintypes.DWORD),
        ("dwSampleRate", wintypes.DWORD),
        ("fHasHorizontalWheel", wintypes.BOOL),
    ]

class RID_DEVICE_INFO(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.DWORD),
        ("dwType", wintypes.DWORD),
        ("keyboard", RID_DEVICE_INFO_KEYBOARD),
        ("mouse", RID_DEVICE_INFO_MOUSE),
    ]

# 低层键盘钩子用 KBDLLHOOKSTRUCT
class KBDLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [
        ("vkCode", wintypes.DWORD),
        ("scanCode", wintypes.DWORD),
        ("flags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]

# 低层鼠标钩子用 MSLLHOOKSTRUCT
class MSLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [
        ("pt_x", wintypes.LONG),
        ("pt_y", wintypes.LONG),
        ("mouseData", wintypes.DWORD),
        ("flags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]

# ── API 声明 ──────────────────────────────────

user32 = ctypes.windll.user32
kernel32 = ctypes.windll.kernel32

# GetRawInputDeviceList
user32.GetRawInputDeviceList.argtypes = [
    ctypes.POINTER(RAWINPUTDEVICELIST),
    ctypes.POINTER(wintypes.UINT),
    wintypes.UINT,
]
user32.GetRawInputDeviceList.restype = wintypes.UINT

# GetRawInputDeviceInfoW
user32.GetRawInputDeviceInfoW.argtypes = [
    wintypes.HANDLE,
    wintypes.UINT,
    wintypes.LPVOID,
    ctypes.POINTER(wintypes.UINT),
]
user32.GetRawInputDeviceInfoW.restype = wintypes.UINT

# SetWindowsHookExW
user32.SetWindowsHookExW.argtypes = [
    ctypes.c_int,
    ctypes.c_void_p,
    wintypes.HINSTANCE,
    wintypes.DWORD,
]
user32.SetWindowsHookExW.restype = wintypes.HHOOK

# UnhookWindowsHookEx
user32.UnhookWindowsHookEx.argtypes = [wintypes.HHOOK]
user32.UnhookWindowsHookEx.restype = wintypes.BOOL

# CallNextHookEx
user32.CallNextHookEx.argtypes = [
    wintypes.HHOOK, ctypes.c_int, wintypes.WPARAM, wintypes.LPARAM,
]
user32.CallNextHookEx.restype = wintypes.LPARAM

# GetMessageW
user32.GetMessageW.argtypes = [
    ctypes.POINTER(wintypes.MSG), wintypes.HWND, wintypes.UINT, wintypes.UINT,
]
user32.GetMessageW.restype = wintypes.BOOL

# GetModuleHandleW
kernel32.GetModuleHandleW.argtypes = [wintypes.LPCWSTR]
kernel32.GetModuleHandleW.restype = wintypes.HINSTANCE

# ── 键码转名称 ────────────────────────────────

# 虚拟键码 → 可读名称（仅常用键）
_VK_NAMES = {
    0x01: "LButton", 0x02: "RButton", 0x04: "MButton",
    0x05: "XButton1", 0x06: "XButton2",
    0x08: "Backspace", 0x09: "Tab", 0x0D: "Enter",
    0x10: "Shift", 0x11: "Ctrl", 0x12: "Alt",
    0x13: "Pause", 0x14: "CapsLock", 0x1B: "Esc",
    0x20: "Space", 0x21: "PgUp", 0x22: "PgDn",
    0x23: "End", 0x24: "Home", 0x25: "Left",
    0x26: "Up", 0x27: "Right", 0x28: "Down",
    0x2D: "Ins", 0x2E: "Del",
    0x5B: "LWin", 0x5C: "RWin", 0x5D: "Menu",
    0x70: "F1", 0x71: "F2", 0x72: "F3", 0x73: "F4",
    0x74: "F5", 0x75: "F6", 0x76: "F7", 0x77: "F8",
    0x78: "F9", 0x79: "F10", 0x7A: "F11", 0x7B: "F12",
}

def vk_to_name(vk_code: int) -> str:
    """虚拟键码 → 可读名称"""
    if vk_code in _VK_NAMES:
        return _VK_NAMES[vk_code]
    if 0x30 <= vk_code <= 0x39:  # 0-9
        return chr(vk_code)
    if 0x41 <= vk_code <= 0x5A:  # A-Z
        return chr(vk_code)
    return f"VK{vk_code:02X}"


# ── 设备枚举 ──────────────────────────────────

def _parse_vid_pid(device_path: str):
    r"""从设备路径 \\?\HID#VID_046D&PID_C31C#... 提取 VID/PID"""
    vid = pid = ""
    m = re.search(r'VID_([0-9A-Fa-f]{4})', device_path)
    if m:
        vid = m.group(1).upper()
    m = re.search(r'PID_([0-9A-Fa-f]{4})', device_path)
    if m:
        pid = m.group(1).upper()
    return vid, pid


def _get_device_name(device_path: str) -> str:
    """通过 CreateFile + HidD_GetProductString 获取设备名称，失败则返回路径最后一段"""
    # 先尝试从路径提取友好名称
    parts = device_path.split("#")
    if len(parts) >= 3:
        return parts[2]  # 路径中的描述段
    return device_path.split("\\")[-1]


def scan_devices():
    """枚举所有键盘和鼠标设备。

    返回: [
        {
            "path": "\\\\?\\HID#VID_046D&PID_C31C#8&...",
            "name": "HID Keyboard Device",
            "vid": "046D",
            "pid": "C31C",
            "type": "keyboard" | "mouse",
        },
        ...
    ]
    """
    # 第一步：获取设备数量
    num_devices = wintypes.UINT(0)
    user32.GetRawInputDeviceList(None, ctypes.byref(num_devices),
                                 ctypes.sizeof(RAWINPUTDEVICELIST))

    if num_devices.value == 0:
        return []

    # 第二步：获取设备列表
    size = ctypes.sizeof(RAWINPUTDEVICELIST)
    devices = (RAWINPUTDEVICELIST * num_devices.value)()
    user32.GetRawInputDeviceList(devices, ctypes.byref(num_devices), size)

    results = []
    for dev in devices[:num_devices.value]:
        # 只关心键盘和鼠标
        if dev.dwType not in (RIM_TYPEKEYBOARD, RIM_TYPEMOUSE):
            continue

        dev_type = "keyboard" if dev.dwType == RIM_TYPEKEYBOARD else "mouse"

        # 第三步：获取设备路径
        name_len = wintypes.UINT(0)
        ret = user32.GetRawInputDeviceInfoW(
            dev.hDevice, RIDI_DEVICENAME, None, ctypes.byref(name_len))
        if ret == 0xFFFFFFFF or name_len.value == 0:
            continue

        name_buf = ctypes.create_unicode_buffer(name_len.value)
        user32.GetRawInputDeviceInfoW(
            dev.hDevice, RIDI_DEVICENAME, name_buf, ctypes.byref(name_len))
        path = name_buf.value

        vid, pid = _parse_vid_pid(path)
        name = _get_device_name(path)

        results.append({
            "path": path,
            "name": name,
            "vid": vid,
            "pid": pid,
            "type": dev_type,
            "handle": dev.hDevice,
        })

    return results


# ── Raw Input 设备识别 ──────────────────────

# WM_INPUT 消息处理常量
WM_INPUT = 0x00FF
WM_DESTROY = 0x0002
RID_INPUT = 0x10000003
RIDEV_INPUTSINK = 0x00000100
RIDEV_REMOVE = 0x00000001

# USB HID Usage Page/Usage
HID_USAGE_PAGE_GENERIC = 0x01
HID_USAGE_GENERIC_KEYBOARD = 0x06
HID_USAGE_GENERIC_MOUSE = 0x02

# Raw Input structures
class RAWINPUTDEVICE(ctypes.Structure):
    _fields_ = [
        ("usUsagePage", wintypes.USHORT),
        ("usUsage", wintypes.USHORT),
        ("dwFlags", wintypes.DWORD),
        ("hwndTarget", wintypes.HWND),
    ]

class RAWINPUTHEADER(ctypes.Structure):
    _fields_ = [
        ("dwType", wintypes.DWORD),
        ("dwSize", wintypes.DWORD),
        ("hDevice", wintypes.HANDLE),
        ("wParam", wintypes.WPARAM),
    ]

class RAWKEYBOARD(ctypes.Structure):
    _fields_ = [
        ("MakeCode", wintypes.USHORT),
        ("Flags", wintypes.USHORT),
        ("Reserved", wintypes.USHORT),
        ("VKey", wintypes.USHORT),
        ("Message", wintypes.UINT),
        ("ExtraInformation", wintypes.ULONG),
    ]
    # RI_KEY_MAKE = 0, RI_KEY_BREAK = 1 (in Flags)

class RAWMOUSE(ctypes.Structure):
    """RAWMOUSE x64：usFlags(2) + 2 padding + usButtonFlags(2) + usButtonData(2) + ulRawButtons(4) + ..."""
    _fields_ = [
        ("usFlags", wintypes.USHORT),          # offset 0, size 2
        ("_pad", wintypes.USHORT),             # offset 2, size 2 (alignment padding)
        ("usButtonFlags", wintypes.USHORT),    # offset 4, size 2
        ("usButtonData", wintypes.USHORT),     # offset 6, size 2
        ("ulRawButtons", wintypes.ULONG),      # offset 8, size 4
        ("lLastX", wintypes.LONG),             # offset 12, size 4
        ("lLastY", wintypes.LONG),             # offset 16, size 4
        ("ulExtraInformation", wintypes.ULONG),# offset 20, size 4
    ]

class RAWINPUT(ctypes.Structure):
    _fields_ = [
        ("header", RAWINPUTHEADER),
        ("data", RAWKEYBOARD),  # Union: use keyboard as default access
    ]

# Window class
class WNDCLASSEXW(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.UINT),
        ("style", wintypes.UINT),
        ("lpfnWndProc", ctypes.c_void_p),
        ("cbClsExtra", ctypes.c_int),
        ("cbWndExtra", ctypes.c_int),
        ("hInstance", wintypes.HINSTANCE),
        ("hIcon", wintypes.HICON),
        ("hCursor", wintypes.HICON),
        ("hbrBackground", wintypes.HBRUSH),
        ("lpszMenuName", wintypes.LPCWSTR),
        ("lpszClassName", wintypes.LPCWSTR),
        ("hIconSm", wintypes.HICON),
    ]

# API declarations
user32.RegisterRawInputDevices.argtypes = [
    ctypes.POINTER(RAWINPUTDEVICE), wintypes.UINT, wintypes.UINT]
user32.RegisterRawInputDevices.restype = wintypes.BOOL

user32.GetRawInputData.argtypes = [
    wintypes.HANDLE, wintypes.UINT, wintypes.LPVOID,
    ctypes.POINTER(wintypes.UINT), wintypes.UINT]
user32.GetRawInputData.restype = wintypes.UINT

user32.CreateWindowExW.argtypes = [
    wintypes.DWORD, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD,
    ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
    wintypes.HWND, wintypes.HMENU, wintypes.HINSTANCE, wintypes.LPVOID]
user32.CreateWindowExW.restype = wintypes.HWND

# 64-bit 安全：WPARAM/LPARAM 用指针宽度类型
_WPARAM = ctypes.c_uint64 if ctypes.sizeof(ctypes.c_void_p) == 8 else wintypes.WPARAM
_LPARAM = ctypes.c_int64 if ctypes.sizeof(ctypes.c_void_p) == 8 else wintypes.LPARAM

user32.DefWindowProcW.argtypes = [wintypes.HWND, wintypes.UINT, _WPARAM, _LPARAM]
user32.DefWindowProcW.restype = _LPARAM

user32.DestroyWindow.argtypes = [wintypes.HWND]
user32.DestroyWindow.restype = wintypes.BOOL

user32.PostQuitMessage.argtypes = [ctypes.c_int]
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
user32.PostQuitMessage.restype = None

user32.DispatchMessageW.argtypes = [ctypes.POINTER(wintypes.MSG)]
user32.DispatchMessageW.restype = wintypes.LPARAM

user32.TranslateMessage.argtypes = [ctypes.POINTER(wintypes.MSG)]
user32.TranslateMessage.restype = wintypes.BOOL

# WNDPROC type
WNDPROC = ctypes.WINFUNCTYPE(_LPARAM, wintypes.HWND, wintypes.UINT, _WPARAM, _LPARAM)

# WH_MOUSE_LL 钩子常量
WH_MOUSE_LL = 14
WM_LBUTTONDOWN = 0x0201
WM_RBUTTONDOWN = 0x0204
WM_MBUTTONDOWN = 0x0207
WM_XBUTTONDOWN = 0x020B
XBUTTON1 = 0x0001
XBUTTON2 = 0x0002

# HOOKPROC 类型: int nCode, WPARAM wParam, LPARAM lParam → LRESULT
HOOKPROC = ctypes.WINFUNCTYPE(_LPARAM, ctypes.c_int, _WPARAM, _LPARAM)

# ── Interception DLL ───────────────────────
_INTERCEPTION_PATH = None
_interception = None  # lazy loaded

def _load_interception():
    """懒加载 interception.dll"""
    global _interception, _INTERCEPTION_PATH
    if _interception is not None:
        return _interception

    candidates = [
        # 新引擎目录（AHI 期望 x64 子目录结构）
        os.path.join(os.path.dirname(__file__), "engines", "ahi", "x64", "interception.dll"),
        # 引擎目录兜底
        os.path.join(os.path.dirname(__file__), "engines", "ahi", "interception.dll"),
        # 应用根目录
        os.path.join(os.path.dirname(__file__), "interception.dll"),
        # 应用目录 Lib\x64 子文件夹
        os.path.join(os.path.dirname(__file__), "Lib", "x64", "interception.dll"),
        # AHI 安装路径
        os.path.join(os.environ.get("USERPROFILE", ""), "Documents", "AutoHotkey", "Lib", "x64", "interception.dll"),
    ]
    for p in candidates:
        if os.path.isfile(p):
            _INTERCEPTION_PATH = p
            break
    if not _INTERCEPTION_PATH:
        return None

    _interception = ctypes.WinDLL(_INTERCEPTION_PATH)

    # 声明函数类型
    INCTX = ctypes.c_void_p
    INDEV = ctypes.c_int
    IS_MOUSE_FN = ctypes.CFUNCTYPE(ctypes.c_int, INDEV)

    _interception.interception_create_context.restype = INCTX
    _interception.interception_destroy_context.argtypes = [INCTX]
    _interception.interception_destroy_context.restype = None
    _interception.interception_is_mouse.argtypes = [INDEV]
    _interception.interception_is_mouse.restype = ctypes.c_int
    _interception.interception_is_keyboard.argtypes = [INDEV]
    _interception.interception_is_keyboard.restype = ctypes.c_int
    _interception.interception_set_filter.argtypes = [INCTX, IS_MOUSE_FN, wintypes.USHORT]
    _interception.interception_set_filter.restype = None
    _interception.interception_wait.argtypes = [INCTX]
    _interception.interception_wait.restype = INDEV
    _interception.interception_receive.argtypes = [INCTX, INDEV, ctypes.c_void_p, wintypes.UINT]
    _interception.interception_receive.restype = ctypes.c_int
    _interception.interception_send.argtypes = [INCTX, INDEV, ctypes.c_void_p, wintypes.UINT]
    _interception.interception_send.restype = ctypes.c_int
    _interception.interception_get_hardware_id.argtypes = [INCTX, INDEV, ctypes.c_void_p, wintypes.UINT]
    _interception.interception_get_hardware_id.restype = wintypes.UINT

    return _interception


# Interception 按钮状态映射
_INTERCEPTION_BTN_MAP = {
    0x001: "LButton", 0x004: "RButton", 0x010: "MButton",
    0x040: "XButton1", 0x100: "XButton2",
}
# 只过滤按钮按下事件（不含移动/滚轮/释放）
_INTERCEPTION_MOUSE_BTN_FILTER = 0x0001 | 0x0004 | 0x0010 | 0x0040 | 0x0100

# InterceptionMouseStroke: state(H) flags(H) rolling(h) x(i) y(i) info(I) = 18 bytes
_INTERCEPTION_STROKE_SIZE = 18


class DeviceIdentifier:
    """在子线程中识别输入设备。

    键盘 → Raw Input (WM_INPUT) [已验证可行]
    鼠标 → Interception DLL (驱动级) [精确到设备]

    callback(device_path: str, key_name: str)
    device_path 匹配 scan_devices() 返回的 path
    """

    def __init__(self, callback):
        self._callback = callback
        self._kb_thread = None
        self._ms_thread = None
        self._running = False
        self._hwnd = None
        self._wndproc_ref = None
        self._ictx = None

        # 构建 hDevice → device_path 映射 (Raw Input 键盘用)
        devices = scan_devices()
        self._handle_to_path = {}
        for d in devices:
            if d.get("handle"):
                self._handle_to_path[d["handle"]] = d["path"]

        # 构建 Interception device_id → GUI device_path 映射 (鼠标用)
        self._idev_to_path = {}
        self._build_interception_map(devices)

    def _build_interception_map(self, devices):
        """通过 Interception 枚举鼠标设备，用 VID/PID 匹配 GUI 设备列表"""
        inter = _load_interception()
        if not inter:
            return

        ctx = inter.interception_create_context()
        if not ctx:
            return

        try:
            for dev_id in range(1, 21):
                if not inter.interception_is_mouse(dev_id):
                    continue
                buf = ctypes.create_unicode_buffer(512)
                ret = inter.interception_get_hardware_id(ctx, dev_id, buf, ctypes.sizeof(buf))
                if ret > 0:
                    hwid = buf.value
                    # 从 hardware ID 提取 VID/PID
                    vid_match = re.search(r'VID_([0-9A-Fa-f]{4})', hwid)
                    pid_match = re.search(r'PID_([0-9A-Fa-f]{4})', hwid)
                    if vid_match and pid_match:
                        vid = vid_match.group(1).upper()
                        pid = pid_match.group(1).upper()
                        # 在 GUI 设备列表中匹配 VID/PID
                        for d in devices:
                            if d.get("type") == "mouse" and d.get("vid") == vid and d.get("pid") == pid:
                                self._idev_to_path[dev_id] = d["path"]
                                break
        finally:
            inter.interception_destroy_context(ctx)

    def start(self):
        if self._running:
            return
        self._running = True
        # 键盘线程: Raw Input
        self._kb_thread = threading.Thread(target=self._raw_input_thread, daemon=True)
        self._kb_thread.start()
        # 鼠标线程: Interception
        inter = _load_interception()
        if inter and self._idev_to_path:
            self._ms_thread = threading.Thread(target=self._interception_thread, daemon=True)
            self._ms_thread.start()

    def stop(self):
        self._running = False
        if self._hwnd:
            user32.PostMessageW(self._hwnd, WM_DESTROY, 0, 0)

    def _raw_input_thread(self):
        """键盘 Raw Input 消息循环"""
        hinst = kernel32.GetModuleHandleW(None)
        self_ref = self

        def _wndproc(hwnd, msg, wparam, lparam):
            if msg == WM_INPUT:
                self_ref._handle_keyboard_raw_input(lparam)
            elif msg == WM_DESTROY:
                user32.PostQuitMessage(0)
                return 0
            return user32.DefWindowProcW(hwnd, msg, wparam, lparam)

        wndproc = WNDPROC(_wndproc)
        self._wndproc_ref = wndproc  # 防止 GC 回收回调

        wc = WNDCLASSEXW()
        wc.cbSize = ctypes.sizeof(WNDCLASSEXW)
        wc.lpfnWndProc = ctypes.cast(wndproc, ctypes.c_void_p)
        wc.hInstance = hinst
        # 每次 start 用唯一类名，避免旧 WNDPROC 悬挂
        cls_name = f"AKRawInput_{id(self)}"
        wc.lpszClassName = cls_name
        user32.RegisterClassExW(ctypes.byref(wc))

        self._hwnd = user32.CreateWindowExW(
            0, cls_name, "", 0, 0, 0, 1, 1,
            None, None, hinst, None)
        if not self._hwnd:
            return

        # 只注册键盘 Raw Input
        rid = RAWINPUTDEVICE()
        rid.usUsagePage = HID_USAGE_PAGE_GENERIC
        rid.usUsage = HID_USAGE_GENERIC_KEYBOARD
        rid.dwFlags = RIDEV_INPUTSINK
        rid.hwndTarget = self._hwnd
        user32.RegisterRawInputDevices(ctypes.byref(rid), 1, ctypes.sizeof(RAWINPUTDEVICE))

        msg = wintypes.MSG()
        while self._running:
            ret = user32.GetMessageW(ctypes.byref(msg), None, 0, 0)
            if ret <= 0:
                break
            user32.DispatchMessageW(ctypes.byref(msg))

        rid.dwFlags = RIDEV_REMOVE
        user32.RegisterRawInputDevices(ctypes.byref(rid), 1, ctypes.sizeof(RAWINPUTDEVICE))
        user32.DestroyWindow(self._hwnd)
        self._hwnd = None

    def _handle_keyboard_raw_input(self, lparam):
        """解析键盘 Raw Input 数据"""
        size = wintypes.UINT(0)
        user32.GetRawInputData(lparam, RID_INPUT, None, ctypes.byref(size),
                               ctypes.sizeof(RAWINPUTHEADER))
        if size.value == 0:
            return

        buf = ctypes.create_string_buffer(size.value)
        if user32.GetRawInputData(lparam, RID_INPUT, buf, ctypes.byref(size),
                                  ctypes.sizeof(RAWINPUTHEADER)) != size.value:
            return

        header = ctypes.cast(buf, ctypes.POINTER(RAWINPUTHEADER)).contents
        hdevice = header.hDevice
        device_path = self._handle_to_path.get(hdevice, "")
        if not device_path:
            return

        if header.dwType != RIM_TYPEKEYBOARD:
            return

        kb = ctypes.cast(
            ctypes.cast(buf, ctypes.c_void_p).value + ctypes.sizeof(RAWINPUTHEADER),
            ctypes.POINTER(RAWKEYBOARD)).contents
        if kb.Flags & 1:  # RI_KEY_BREAK, skip release
            return
        key_name = vk_to_name(kb.VKey)
        try:
            self._callback(device_path, key_name)
        except Exception:
            pass

    def _interception_thread(self):
        """Interception 鼠标识别循环"""
        inter = _load_interception()
        if not inter:
            return

        ctx = inter.interception_create_context()
        if not ctx:
            return

        # 设置鼠标过滤：只接收按钮按下事件
        is_mouse_pred = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_int)(lambda d: inter.interception_is_mouse(d))
        inter.interception_set_filter(ctx, is_mouse_pred, _INTERCEPTION_MOUSE_BTN_FILTER)

        stroke_buf = ctypes.create_string_buffer(_INTERCEPTION_STROKE_SIZE)

        while self._running:
            dev = inter.interception_wait(ctx)
            if dev <= 0:
                continue
            if not inter.interception_is_mouse(dev):
                continue

            ret = inter.interception_receive(ctx, dev, stroke_buf, 1)
            if ret <= 0:
                continue

            # 解析 InterceptionMouseStroke: state(H) flags(H) rolling(h) x(i) y(i) info(I)
            raw = stroke_buf.raw[:_INTERCEPTION_STROKE_SIZE]
            state, _flags, _rolling, _x, _y, _info = struct.unpack_from('<HHhiiI', raw, 0)

            btn = _INTERCEPTION_BTN_MAP.get(state & 0x1FF, "")
            if btn:
                device_path = self._idev_to_path.get(dev, "")
                if device_path:
                    try:
                        self._callback(device_path, btn)
                    except Exception:
                        pass

            # 关键：放行鼠标事件，否则系统收不到点击
            inter.interception_send(ctx, dev, stroke_buf, 1)

        inter.interception_destroy_context(ctx)



# ── 测试入口 ──────────────────────────────────
if __name__ == "__main__":
    print("=== 扫描设备 ===")
    devices = scan_devices()
    for d in devices:
        print(f"  [{d['type']:>8}] {d['name']:<40} VID:{d['vid']} PID:{d['pid']}")

    print("\n=== 设备识别测试（5秒）===")
    print("  键盘 → Raw Input | 鼠标 → Interception DLL")
    def on_event(path, key):
        short = path.split("\\")[-1][:30] if path else "?"
        print(f"  [{short}] {key}")

    identifier = DeviceIdentifier(on_event)
    identifier.start()
    import time
    time.sleep(5)
    identifier.stop()
    print("done.")
