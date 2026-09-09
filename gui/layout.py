"""
键盘布局定义 — ANSI 104 键全键盘
提供按键位置数据，供 Canvas 绘制键盘图。
"""

# ── 布局常量 ──
KW = 44   # 标准键宽
KH = 44   # 标准键高
GAP = 2   # 键间距
PITCH = KW + GAP  # 标准步进 = 46

def _k(x, y, w=KW, h=KH):
    """简写：创建按键位置字典"""
    return {"x": x, "y": y, "w": w, "h": h}

# ── 全键盘布局 (ANSI 104) ──
# key_id 对应规范化后的键名
# label 是显示文本，可以是 "A" 或 ("!\n1",)

FULL_KEYBOARD = {
    # ═══ 功能键行 ═══
    "Esc":     _k(0, 0, 52, 42),
    "F1":      _k(64, 0, 44, 44),
    "F2":      _k(110, 0, 44, 44),
    "F3":      _k(156, 0, 44, 44),
    "F4":      _k(202, 0, 44, 44),
    "F5":      _k(270, 0, 44, 44),
    "F6":      _k(316, 0, 44, 44),
    "F7":      _k(362, 0, 44, 44),
    "F8":      _k(408, 0, 44, 44),
    "F9":      _k(476, 0, 44, 44),
    "F10":     _k(522, 0, 44, 44),
    "F11":     _k(568, 0, 44, 44),
    "F12":     _k(614, 0, 60, 44),
    "PrintScreen": _k(706, 0, 44, 44),
    "ScrollLock":  _k(752, 0, 44, 44),
    "Pause":   _k(798, 0, 44, 44),

    # ═══ 数字行 ═══
    "`":       _k(0, 60, 44, 44),
    "1":       _k(46, 60),
    "2":       _k(92, 60),
    "3":       _k(138, 60),
    "4":       _k(184, 60),
    "5":       _k(230, 60),
    "6":       _k(276, 60),
    "7":       _k(322, 60),
    "8":       _k(368, 60),
    "9":       _k(414, 60),
    "0":       _k(460, 60),
    "-":       _k(506, 60),
    "=":       _k(552, 60),
    "Backspace": _k(598, 60, 76, 44),
    "Insert":  _k(706, 60),
    "Home":    _k(752, 60),
    "PgUp":    _k(798, 60),

    # ═══ QWERTY 行 ═══
    "Tab":     _k(0, 106, 60, 44),
    "q":       _k(62, 106),
    "w":       _k(108, 106),
    "e":       _k(154, 106),
    "r":       _k(200, 106),
    "t":       _k(246, 106),
    "y":       _k(292, 106),
    "u":       _k(338, 106),
    "i":       _k(384, 106),
    "o":       _k(430, 106),
    "p":       _k(476, 106),
    "[":       _k(522, 106),
    "]":       _k(568, 106),
    "\\":      _k(614, 106, 60, 44),
    "Delete":  _k(706, 106),
    "End":     _k(752, 106),
    "PgDn":    _k(798, 106),

    # ═══ 主行 (Home Row) ═══
    "CapsLock": _k(0, 152, 76, 44),
    "a":       _k(78, 152),
    "s":       _k(124, 152),
    "d":       _k(170, 152),
    "f":       _k(216, 152),
    "g":       _k(262, 152),
    "h":       _k(308, 152),
    "j":       _k(354, 152),
    "k":       _k(400, 152),
    "l":       _k(446, 152),
    ";":       _k(492, 152),
    "'":       _k(538, 152),
    "Enter":   _k(584, 152, 90, 44),

    # ═══ Shift 行 ═══
    "LShift":  _k(0, 198, 98, 44),
    "z":       _k(100, 198),
    "x":       _k(146, 198),
    "c":       _k(192, 198),
    "v":       _k(238, 198),
    "b":       _k(284, 198),
    "n":       _k(330, 198),
    "m":       _k(376, 198),
    ",":       _k(422, 198),
    ".":       _k(468, 198),
    "/":       _k(514, 198),
    "RShift":  _k(562, 198, 112, 44),
    "Up":      _k(752, 198),

    # ═══ 底行 ═══
    "LCtrl":   _k(0, 244, 69, 44),
    "LWin":    _k(71, 244, 57, 44),
    "LAlt":    _k(130, 244, 57, 44),
    "Space":   _k(189, 244, 252, 44),
    "RAlt":    _k(443, 244, 57, 44),
    "RWin":    _k(502, 244, 57, 44),
    "AppsKey": _k(561, 244, 46, 44),
    "RCtrl":   _k(609, 244, 65, 44),
    "Left":    _k(706, 244),
    "Down":    _k(752, 244),
    "Right":   _k(798, 244),

    # ═══ 数字键盘 ═══
    "NumLock":    _k(874, 60),
    "NumpadDiv":  _k(920, 60),
    "NumpadMult": _k(966, 60),
    "NumpadSub":  _k(1012, 60),

    "Numpad7": _k(874, 106),
    "Numpad8": _k(920, 106),
    "Numpad9": _k(966, 106),
    "NumpadAdd": _k(1012, 106, 44, 90),

    "Numpad4": _k(874, 152),
    "Numpad5": _k(920, 152),
    "Numpad6": _k(966, 152),

    "Numpad1": _k(874, 198),
    "Numpad2": _k(920, 198),
    "Numpad3": _k(966, 198),
    "NumpadEnter": _k(1012, 198, 44, 90),

    "Numpad0": _k(874, 244, 90, 44),
    "NumpadDot": _k(966, 244),
}

# 键盘图总尺寸（用于 Canvas scrollregion）
# 键盘 ~1056 + 按钮区 91 + 余量
KB_WIDTH = 1180
KB_HEIGHT = 292

# ── 鼠标按钮区域 ──
MOUSE_X = 1080
MOUSE_Y = 6


# ── 显示标签映射 ──
_LABEL_MAP = {
    # 数字行：显示双标签
    "`": "~\n`", "1": "!\n1", "2": "@\n2", "3": "#\n3",
    "4": "$\n4", "5": "%\n5", "6": "^\n6", "7": "&\n7",
    "8": "*\n8", "9": "(\n9", "0": ")\n0",
    "-": "_\n-", "=": "+\n=",
    "[": "{\n[", "]": "}\n]", "\\": "|\n\\",
    ";": ":\n;", "'": "\"\n'",
    ",": "<\n,", ".": ">\n.", "/": "?\n/",
    # 修饰键：短名
    "Backspace": "Bksp",
    "CapsLock": "Caps",
    "Enter": "Enter",
    "LShift": "Shift",
    "RShift": "Shift",
    "LCtrl": "Ctrl",
    "RCtrl": "Ctrl",
    "LAlt": "Alt",
    "RAlt": "Alt",
    "LWin": "Win",
    "RWin": "Win",
    "AppsKey": "Menu",
    "PrintScreen": "PrSc",
    "ScrollLock": "ScLk",
    "Insert": "Ins",
    "Delete": "Del",
    "PgUp": "PgUp",
    "PgDn": "PgDn",
    "Home": "Home",
    "End": "End",
    "Up": "\u2191",
    "Down": "\u2193",
    "Left": "\u2190",
    "Right": "\u2192",
    "NumLock": "NumLk",
    "NumpadDiv": "/",
    "NumpadMult": "*",
    "NumpadSub": "-",
    "NumpadAdd": "+",
    "NumpadEnter": "Enter",
    "NumpadDot": ".",
    "Numpad0": "0",
    "Numpad1": "1",
    "Numpad2": "2",
    "Numpad3": "3",
    "Numpad4": "4",
    "Numpad5": "5",
    "Numpad6": "6",
    "Numpad7": "7",
    "Numpad8": "8",
    "Numpad9": "9",
    "Space": "",
    # 鼠标键
    "MouseLeft":   "左键",
    "MouseRight":  "右键",
    "MouseMiddle": "中键",
    "MouseSide1":  "侧1",
    "MouseSide2":  "侧2",
    "WheelUp":     "滚上",
    "WheelDown":   "滚下",
}


def get_label(key_id):
    """返回按键的显示标签，未映射的返回原 key_id 的大写"""
    if key_id in _LABEL_MAP:
        return _LABEL_MAP[key_id]
    # 单字母键：显示大写
    if len(key_id) == 1 and key_id.isalpha():
        return key_id.upper()
    return key_id


def get_all_key_ids():
    """返回布局中所有 key_id 的列表"""
    return list(FULL_KEYBOARD.keys())


def get_key_ids_by_category():
    """按区域分类返回 key_id 列表"""
    alpha = []      # 字母区
    num_row = []    # 数字行
    func = []       # 功能键
    mods = []       # 修饰键
    nav = []        # 导航区
    numpad = []     # 数字键盘
    other = []      # 其他

    func_keys = {"Esc", "F1", "F2", "F3", "F4", "F5", "F6", "F7",
                 "F8", "F9", "F10", "F11", "F12",
                 "PrintScreen", "ScrollLock", "Pause"}
    mod_keys = {"LShift", "RShift", "LCtrl", "RCtrl", "LAlt", "RAlt",
                "LWin", "RWin", "AppsKey", "CapsLock", "Tab",
                "Backspace", "Enter", "Space"}
    nav_keys = {"Insert", "Home", "PgUp", "Delete", "End", "PgDn",
                "Up", "Down", "Left", "Right"}
    numpad_keys = {k for k in FULL_KEYBOARD if k.startswith("Numpad") or k == "NumLock"}

    for kid in FULL_KEYBOARD:
        if kid in func_keys:
            func.append(kid)
        elif kid in mod_keys:
            mods.append(kid)
        elif kid in nav_keys:
            nav.append(kid)
        elif kid in numpad_keys:
            numpad.append(kid)
        elif kid.isalpha() and len(kid) == 1:
            alpha.append(kid)
        elif kid.isdigit() or kid in {"`", "-", "=", "[", "]", "\\", ";", "'", ",", ".", "/"}:
            num_row.append(kid)
        else:
            other.append(kid)

    return {
        "alpha": sorted(alpha, key=lambda k: k.lower()),
        "num_row": num_row,
        "func": func,
        "mods": mods,
        "nav": nav,
        "numpad": numpad,
        "other": other,
    }
