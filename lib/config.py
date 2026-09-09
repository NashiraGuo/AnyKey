"""
AnyKey - 配置管理模块
负责加载、保存、验证配置
"""

import json
import os
import sys
import re
import subprocess

# ──────────────────────────────────────────────
# 已移除 DEFAULT_CONFIG：读取空 config 时不再自动回填示例 combo/层/tapDance，
# 避免「检索不到就默认填写」污染用户 config（详见 load_config：文件缺失时返回 {}）。
# 需要空结构底座的地方（leader/layers）改用下面的极简结构模板；这些模板只含
# 结构字段与合理标量，不含任何示例映射，绝不会凭空造出 combo/层条目。

# leader 结构底座：仅 timeoutMs / loopCapture / sequences 三个结构字段（无示例序列）
EMPTY_LEADER = {
    "timeoutMs": 2000,     # 公共滑动窗口超时（毫秒）
    "loopCapture": False,  # 循环捕获（默认关，维持原版）
    "sequences": [],       # 每条序列：{"keys":["{a}","{b}"], "output":"...", "timeoutMs":0}
}

# layers 结构底座：仅结构字段（无示例 baseLayer/layerDefs 映射）
EMPTY_LAYERS = {}
# 打包成 exe 后 __file__ 指向临时目录，exe 在根目录
if getattr(sys, "frozen", False):
    _BASE_DIR = os.path.dirname(os.path.abspath(sys.executable))  # dist/
    _BUNDLE_DIR = getattr(sys, "_MEIPASS", _BASE_DIR)  # PyInstaller 打包资源目录
else:
    _BASE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # 项目根目录
    _BUNDLE_DIR = os.path.join(_BASE_DIR, "assets")

CONFIG_FILE = os.path.join(_BASE_DIR, "anykey_config.json")
RUST_ENGINE_PATH = os.path.join(_BASE_DIR, "engines", "rust", "anykey-engine.exe")


def check_rust_installed():
    """检查 Rust 引擎 exe 是否存在"""
    return os.path.isfile(RUST_ENGINE_PATH)

# AHK 不可用时的后备规范化映射（GetKeyName 返回值 → 标准小写）
_KEY_NORM_FALLBACK = {
    "esc": "escape", "escape": "escape",
    "bs": "backspace", "backspace": "backspace",
    "return": "enter", "enter": "enter",
    "ins": "insert", "insert": "insert",
    "del": "delete", "delete": "delete",
    "pgup": "pgup", "pageup": "pgup",
    "pgdn": "pgdn", "pagedown": "pgdn",
    "prtsc": "printscreen", "printscreen": "printscreen",
    "scrlk": "scrolllock", "scrolllock": "scrolllock",
    "capslk": "capslock", "capslock": "capslock",
    "ctrl": "lcontrol", "control": "lcontrol",
    "shift": "lshift",
    "alt": "lalt",
    "win": "lwin",
    "apps": "appskey", "appskey": "appskey",
    # 鼠标键（驱动统一为 lowercase 规范名：mouseleft/mouseright/...）
    "mouseleft": "mouseleft", "mouseright": "mouseright",
    "mousemiddle": "mousemiddle",
    "mouseside1": "mouseside1", "mouseside2": "mouseside2",
    "wheelup": "wheelup", "wheeldown": "wheeldown",
}

# ── 键名规范化缓存 ──────────────────────────
_KEY_NORM_CACHE = None  # None=未加载, dict=已加载

def _get_norm_map():
    """生成键名规范化映射表（静态，不依赖 AHK）"""
    result = dict(_KEY_NORM_FALLBACK)
    # 自映射：常用键名
    for c in range(26):
        k = chr(ord('a') + c)
        result[k] = k
    for i in range(10):
        result[str(i)] = str(i)
    for i in range(1, 25):
        result[f"f{i}"] = f"f{i}"
    for i in range(10):
        result[f"numpad{i}"] = f"numpad{i}"
    for k in ["numpadadd","numpadsub","numpadmul","numpaddot","numpadenter",
              "numpaddiv","numpadins","numpadend","numpaddown","numpadpgdn",
              "numpadleft","numpadclear","numpadright","numpadhome","numpadup","numpadpgup"]:
        result[k] = k
    for k in ["lcontrol","rcontrol","lshift","rshift","lalt","ralt","lwin","rwin"]:
        result[k] = k
    for k in ["tab","space","home","end","up","down","left","right",
              "pause","break","numlock","-","=","[","]","\\\\",";","'","`",",",".","/"]:
        result[k] = k
    for k in ["mouseleft","mouseright","mousemiddle","mouseside1","mouseside2",
              "wheelup","wheeldown"]:
        result[k] = k
    # 缩写映射（在 _KEY_NORM_FALLBACK 基础上补充）
    result["ctrl"] = "lcontrol"
    result["control"] = "lcontrol"
    result["shift"] = "lshift"
    result["alt"] = "lalt"
    result["win"] = "lwin"
    return result

def _norm_key(key):
    """规范化键名：esc → escape, NumpadAdd → numpadadd"""
    global _KEY_NORM_CACHE
    if _KEY_NORM_CACHE is None:
        _KEY_NORM_CACHE = _get_norm_map()
    k = key.strip().lower()
    return _KEY_NORM_CACHE.get(k, k)
def load_config():
    if os.path.exists(CONFIG_FILE):
        with open(CONFIG_FILE, "r", encoding="utf-8") as f:
            return json.load(f)
    return {}

def save_config(cfg):
    with open(CONFIG_FILE, "w", encoding="utf-8") as f:
        json.dump(cfg, f, ensure_ascii=False, indent=2)

def _is_single_key(output: str) -> bool:
    """判断输出是否为单键（用于保持型 Combo）
    单键：{xxx} 格式且只有一个，或纯单字符
    多键：包含多个 {} 或纯文本字符串
    """
    # 找出所有 {xxx} 格式的按键
    matches = re.findall(r'\{[^}]+\}', output)
    # 单键：只有一个 {xxx} 且没有额外文本
    if len(matches) == 1:
        return output.strip() == matches[0]


def _is_func_call(output: str) -> bool:
    """检测输出是否是 AHK 函数调用语法，兼容 {func()} 包裹写法"""
    o = output.strip()
    if o.startswith("{") and o.endswith("}"):
        o = o[1:-1]
    return bool(re.match(r'^[A-Za-z_]\w*\(.+\)$', o))


# ── 输出值归一化（单键名包裹）─────────────────
# 与 anykey-engine/src/util.rs 的 wrap_single_key_output 规则保持一致：
#   - 空 → 空
#   - 已 {X} → 规范化内部键名（缩写→全名、全名转小写），如 {pgdn}→{pagedown}、{bs}→{backspace}
#   - 裸单字符键名（小写字母/数字/基础符号）→ 包成 {X}
#   - 其余（多字符裸值/文本/宏）→ 原样
# 裸键名判定：仅一个字符，且为 a-z / 0-9 / 键盘基础符号（无 Shift）；
# 大写字母、Shift 符号、多字符一律视为文本（用户 2026-07-08 规则）。
_KEY_ALIAS_TO_FULL = {
    "esc": "escape",
    "del": "delete",
    "ins": "insert",
    "pgup": "pageup",
    "pgdn": "pagedown",
    "prtsc": "printscreen",
    "scrlk": "scrolllock",
    "capslk": "capslock",
    "appskey": "apps",
    "bs": "backspace",
    # 鼠标键：旧名 → 新规范名（迁移，2026-08-06）
    "lbutton": "mouseleft",
    "rbutton": "mouseright",
    "mbutton": "mousemiddle",
    "xbutton1": "mouseside1",
    "xbutton2": "mouseside2",
    # 小键盘乘号：速查栏/习惯写法 → 引擎规范名
    "numpadmult": "numpadmul",
}

# 键盘上不需要 Shift 的基础符号（与 util.rs is_basic_symbol 一致）
_BASIC_SYMBOLS = set("`-=[]\\;',./")

def _is_wrapped_key_output(s: str) -> bool:
    return len(s) >= 3 and s.startswith("{") and s.endswith("}")


def _escape_spaces_outside_braces(val):
    """只把 { ... } 宏 token 之外的字面空格替换为 {space}。

    带参宏（如 {Select 12} / {Sleep 200}）内部的空格必须原样保留，
    否则会被误伤成 {select{space}12}，导致引擎 send_key 的 Select/Repeat
    正则（只认字面空白）无法识别、再被通用分段器贪婪错拆。

    用配对括号深度识别宏区段：{ 深度+1，} 深度-1；深度>0 视为宏内部。
    未配对的多余 } 在深度 0 时按普通字符处理（不吞、不替换）。
    """
    out = []
    depth = 0
    for c in val:
        if c == '{':
            depth += 1
            out.append(c)
        elif c == '}':
            if depth > 0:
                depth -= 1
            out.append(c)
        elif c == ' ' and depth == 0:
            out.append('{space}')
        else:
            out.append(c)
    return ''.join(out)


def normalize_key_output(val):
    """归一化单个输出值（规则见 _KEY_ALIAS_TO_FULL / _BASIC_SYMBOLS 上方说明）。"""
    if not val:
        return val
    if _is_wrapped_key_output(val):
        inner = val[1:-1].strip()
        # 内部含空格/嵌套 {} 视为宏或复合输出，不改造（保持原样）
        if " " in inner or "{" in inner or "}" in inner:
            return val
        full = _KEY_ALIAS_TO_FULL.get(inner.lower(), inner.lower())
        return "{" + full + "}"
    # 裸文本值：保留用户写入的空格。空格字符替换为 {space} 以避免引擎 drain 端 trim 误杀。
    # 但 RUN: 宏里的空格是命令参数分隔符，不能转义；{...} 宏 token 内部的空格（如 {select 12}）也须保留。
    if ' ' in val and not val.startswith("RUN:"):
        val = _escape_spaces_outside_braces(val)
    if len(val) == 1:
        c = val[0]
        if ('a' <= c <= 'z') or ('0' <= c <= '9') or (c in _BASIC_SYMBOLS):
            return "{" + val + "}"
    return val

def normalize_layers_key_outputs(cfg):
    """归一化 tapDance 中所有层的 tap/hold/dt/dh 字段为 {X} 格式。扁平格式。"""
    layers = cfg.get("tapDance")
    if not isinstance(layers, dict):
        return
    fields = ("tap", "hold", "dt", "dh")
    for ln, km in layers.items():
        if not isinstance(km, dict) or ln in ("holdTerm", "doubleTapTerm", "doubleHoldTerm"):
            continue
        for entry in km.values():
            if isinstance(entry, dict):
                for f in fields:
                    if entry.get(f):
                        entry[f] = normalize_key_output(entry[f])


# ── 层名称映射 ─────────────────────────────
# GUI 显示数字 0=base, 1=fn1, 2=fn2, ...
# 内部存储标准化名称: "base", "fn1", "fn2", ...

def _layer_display(layer_name: str) -> str:
    """层名称 → 显示数字。'base'→'0', 'fn1'→'1', 'fn2'→'2'"""
    if layer_name == "base" or layer_name == "0":
        return "0"
    if layer_name.startswith("fn") and layer_name[2:].isdigit():
        return str(int(layer_name[2:]))
    return layer_name

def _layer_from_display(val: str) -> str:
    """显示数字 → 层名称。'0'→'base', '1'→'fn1', '2'→'fn2'"""
    if val == "0":
        return "base"
    if val.isdigit():
        n = int(val)
        if n > 0:
            return f"fn{n}"
    return val  # 非数字直接透传（兼容旧配置）

