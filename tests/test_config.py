"""test_config.py — 系统性测试用配置

设计原则：
- TapDance 组合全覆盖：全配/缺hold/缺dt/缺dh/仅tap+dt/仅tap+dh/仅hold+dh/仅tap
- 同名键跨层不同 TD 配置
- 键同时是 combo 键和 TD 键
- 层切换、修饰键、宏混合存在
"""

TEST_CONFIG = {
    "comboTime": 200,
    "comboMap": [
        # ── 纯 passthrough 键的 combo ──
        {"key1": "q", "key2": "w", "output": "{Esc}", "layer": "base"},
        {"key1": "z", "key2": "x", "output": "{Backspace}", "layer": "base"},
        # ── TD 键也参与的组合 ──
        {"key1": "a", "key2": "s", "output": "{Left}", "layer": "base"},
        # ── fn1 层的 combo ──
        {"key1": "h", "key2": "j", "output": "{Home}", "layer": "fn1"},
        # ── fn2 层的 combo ──
        {"key1": "k", "key2": "l", "output": "{End}", "layer": "fn2"},
    ],
    "layers": {
        "enabled": True,
        "baseLayer": {
            # ── A: 全配 TD ──
            "a": {
                "tap": "{a}", "hold": "{fn1}", "ht": "200",
                "dt": "{F1}",   "dtt": "250",
                "dh": "{F2}",   "dht": "200",
            },
            # ── B: 缺 doubleTap ──
            "s": {
                "tap": "{s}", "hold": "{fn2}", "ht": "200",
                "dt": "",      "dtt": "",
                "dh": "{F3}",  "dht": "200",
            },
            # ── C: 缺 doubleHold ──
            "d": {
                "tap": "{d}", "hold": "{shift}", "ht": "200",
                "dt": "{F4}", "dtt": "250",
                "dh": "",     "dht": "",
            },
            # ── D: 缺 hold ──
            "f": {
                "tap": "{f}", "hold": "",       "ht": "",
                "dt": "{F5}", "dtt": "250",
                "dh": "{F6}", "dht": "200",
            },
            # ── E: 仅 tap+dt ──
            "g": {
                "tap": "{g}", "hold": "",   "ht": "",
                "dt": "{F7}", "dtt": "250",
                "dh": "",     "dht": "",
            },
            # ── F: 仅 tap+dh ──
            "h": {
                "tap": "{h}", "hold": "",   "ht": "",
                "dt": "",     "dtt": "",
                "dh": "{F8}", "dht": "200",
            },
            # ── G: 仅 hold+dh ──
            "j": {
                "tap": "",     "hold": "{fn3}", "ht": "200",
                "dt": "",      "dtt": "",
                "dh": "{F9}",  "dht": "200",
            },
            # ── H: 仅 tap (无 TD 行为) ──
            "k": {
                "tap": "{k}", "hold": "", "ht": "",
                "dt": "",     "dtt": "",
                "dh": "",     "dht": "",
            },
            # ── I: 仅 hold (层切换键) ──
            "l": {
                "tap": "",     "hold": "{fn1}", "ht": "200",
                "dt": "",      "dtt": "",
                "dh": "",      "dht": "",
            },
            # ── Z: 修饰键 hold (非层键，测试 auto-repeat holding 行为) ──
            "z": {
                "tap": "{z}", "hold": "{shift}", "ht": "200",
                "dt": "",     "dtt": "",
                "dh": "",     "dht": "",
            },
            # ── J: 全配 + 宏输出 ──
            "space": {
                "tap": "{Space}", "hold": "{fn1}",  "ht": "200",
                "dt": "{CapsLock}",  "dtt": "250",
                "dh": "",           "dht": "",
            },
        },
        "switchKeys": [],
        "layerDefs": [
            # ═══ fn1 ═══
            {
                "name": "fn1",
                "blockHold": False,
                "keyMap": {
                    # a: 同名键，不同 TD（仅 tap+hold→ctrl，无 dt/dh）
                    "a": {
                        "tap": "A", "hold": "{ctrl}", "ht": "200",
                        "dt": "",   "dtt": "",
                        "dh": "",   "dht": "",
                    },
                    # h: 仅 tap (fn1 内是方向键)
                    "h": {
                        "tap": "{Left}", "hold": "", "ht": "",
                        "dt": "",        "dtt": "",
                        "dh": "",        "dht": "",
                    },
                    # j: 仅 tap (fn1 内也是方向键)
                    "j": {
                        "tap": "{Down}", "hold": "", "ht": "",
                        "dt": "",        "dtt": "",
                        "dh": "",        "dht": "",
                    },
                },
            },
            # ═══ fn2 ═══
            {
                "name": "fn2",
                "blockHold": False,
                "keyMap": {
                    # s: 同名键，仅 tap→宏文本
                    "s": {
                        "tap": "hello ", "hold": "", "ht": "",
                        "dt": "",         "dtt": "",
                        "dh": "",         "dht": "",
                    },
                    # k: 同名键，fn2 内→方向键
                    "k": {
                        "tap": "{Right}", "hold": "", "ht": "",
                        "dt": "",         "dtt": "",
                        "dh": "",         "dht": "",
                    },
                    # l: 同名键，fn2 内→方向键
                    "l": {
                        "tap": "{Up}", "hold": "", "ht": "",
                        "dt": "",      "dtt": "",
                        "dh": "",      "dht": "",
                    },
                },
            },
            # ═══ fn3 ═══
            {
                "name": "fn3",
                "blockHold": False,
                "keyMap": {
                    # j: 同名键，fn3 内→方向键
                    "j": {
                        "tap": "{Up}", "hold": "", "ht": "",
                        "dt": "",      "dtt": "",
                        "dh": "",      "dht": "",
                    },
                },
            },
            # ═══ fn4: 宏/修饰键测试 ═══
            {
                "name": "fn4",
                "blockHold": False,
                "keyMap": {
                    "1": {
                        "tap": "RUN:notepad.exe", "hold": "", "ht": "",
                        "dt": "", "dtt": "", "dh": "", "dht": "",
                    },
                    "2": {
                        "tap": "^c", "hold": "", "ht": "",
                        "dt": "", "dtt": "", "dh": "", "dht": "",
                    },
                },
            },
        ],
    },
    "tapDance": {
        "enabled": True,
        "holdTerm": 200,
        "doubleTapTerm": 250,
        "doubleHoldTerm": 200,
        "keys": {},
    },
    "flowControl": {
        "comboToLayer": False,
        "comboToTapDance": False,
    },
    "subscribed_devices": [],
}


# ═══════════════════════════════════════════════════
# 基于此 config 的测试场景
# ═══════════════════════════════════════════════════

TEST_SCENARIOS = [
    # ── A: 全配 TD 的各种行为 ──
    {"name": "td_full_tap_a", "desc": "A: 全配TD tap→{a}",
     "steps": [{"dn": "a"}, {"up": "a"}, {"wait": 500}]},
    {"name": "td_full_hold_a", "desc": "A: 全配TD hold→{fn1}",
     "steps": [{"dn": "a"}, {"wait": 500}, {"up": "a"}, {"wait": 500}]},
    {"name": "td_full_dt_a", "desc": "A: 全配TD doubleTap→{F1}",
     "steps": [{"dn": "a"}, {"up": "a"}, {"dn": "a"}, {"up": "a"}, {"wait": 500}]},

    # ── B: 缺 doubleTap ──
    {"name": "td_nodt_tap_s", "desc": "B: 缺dt TD tap→{s}",
     "steps": [{"dn": "s"}, {"up": "s"}, {"wait": 500}]},
    {"name": "td_nodt_hold_s", "desc": "B: 缺dt TD hold→{fn2}",
     "steps": [{"dn": "s"}, {"wait": 500}, {"up": "s"}, {"wait": 500}]},
    {"name": "td_nodt_dh_s", "desc": "B: 缺dt TD doubleHold→{F3}",
     "steps": [{"dn": "s"}, {"up": "s"}, {"dn": "s"}, {"wait": 300}, {"up": "s"}, {"wait": 500}]},

    # ── C: 缺 doubleHold (hold→shift修饰键) ──
    {"name": "td_nodh_tap_d", "desc": "C: 缺dh TD tap→{d}",
     "steps": [{"dn": "d"}, {"up": "d"}, {"wait": 500}]},
    {"name": "td_nodh_hold_d", "desc": "C: 缺dh TD hold→{shift}",
     "steps": [{"dn": "d"}, {"wait": 300}, {"up": "d"}, {"wait": 500}]},
    {"name": "td_nodh_dt_d", "desc": "C: 缺dh TD doubleTap→{F4}",
     "steps": [{"dn": "d"}, {"up": "d"}, {"dn": "d"}, {"up": "d"}, {"wait": 500}]},

    # ── D: 缺 hold ──
    {"name": "td_nohold_tap_f", "desc": "D: 缺hold TD tap→{f}",
     "steps": [{"dn": "f"}, {"up": "f"}, {"wait": 500}]},
    {"name": "td_nohold_dt_f", "desc": "D: 缺hold TD doubleTap→{F5}",
     "steps": [{"dn": "f"}, {"up": "f"}, {"dn": "f"}, {"up": "f"}, {"wait": 500}]},
    {"name": "td_nohold_dh_f", "desc": "D: 缺hold TD doubleHold→{F6}",
     "steps": [{"dn": "f"}, {"up": "f"}, {"dn": "f"}, {"wait": 300}, {"up": "f"}, {"wait": 500}]},

    # ── E: 仅 tap+dt ──
    {"name": "td_tapdt_tap_g", "desc": "E: 仅tap+dt tap→{g}",
     "steps": [{"dn": "g"}, {"up": "g"}, {"wait": 500}]},
    {"name": "td_tapdt_dt_g", "desc": "E: 仅tap+dt doubleTap→{F7}",
     "steps": [{"dn": "g"}, {"up": "g"}, {"dn": "g"}, {"up": "g"}, {"wait": 500}]},

    # ── F: 仅 tap+dh ──
    {"name": "td_tapdh_tap_h", "desc": "F: 仅tap+dh tap→{h}",
     "steps": [{"dn": "h"}, {"up": "h"}, {"wait": 500}]},
    {"name": "td_tapdh_dh_h", "desc": "F: 仅tap+dh doubleHold→{F8}",
     "steps": [{"dn": "h"}, {"up": "h"}, {"dn": "h"}, {"wait": 300}, {"up": "h"}, {"wait": 500}]},

    # ── G: 仅 hold+dh ──
    {"name": "td_holddh_hold_j", "desc": "G: 仅hold+dh hold→{fn3}",
     "steps": [{"dn": "j"}, {"wait": 500}, {"up": "j"}, {"wait": 500}]},
    {"name": "td_holddh_dh_j", "desc": "G: 仅hold+dh doubleHold→{F9}",
     "steps": [{"dn": "j"}, {"up": "j"}, {"dn": "j"}, {"wait": 300}, {"up": "j"}, {"wait": 500}]},

    # ── H: 仅 tap (无 TD 行为) ──
    {"name": "plain_tap_k", "desc": "H: 仅tap 的键→{k}",
     "steps": [{"dn": "k"}, {"up": "k"}, {"wait": 500}]},

    # ── I: 仅 hold (层切换) ──
    {"name": "plain_hold_l", "desc": "I: 仅hold 层切换→{fn1}",
     "steps": [{"dn": "l"}, {"wait": 500}, {"up": "l"}, {"wait": 500}]},

    # ── J: Space 全配 ──
    {"name": "td_space_tap", "desc": "Space tap→{Space}",
     "steps": [{"dn": "space"}, {"up": "space"}, {"wait": 500}]},
    {"name": "td_space_hold", "desc": "Space hold→{fn1}",
     "steps": [{"dn": "space"}, {"wait": 300}, {"up": "space"}, {"wait": 500}]},
    {"name": "td_space_dt", "desc": "Space doubleTap→{CapsLock}",
     "steps": [{"dn": "space"}, {"up": "space"}, {"dn": "space"}, {"up": "space"}, {"wait": 500}]},

    # ── Combo: 纯 passthrough 键 ──
    {"name": "combo_qw_esc", "desc": "combo q+w→Esc (非TD键)",
     "steps": [{"dn": "q"}, {"dn": "w"}, {"up": "w"}, {"up": "q"}, {"wait": 500}]},
    {"name": "combo_zx_bs", "desc": "combo z+x→Backspace (非TD键)",
     "steps": [{"dn": "z"}, {"dn": "x"}, {"up": "x"}, {"up": "z"}, {"wait": 500}]},

    # ── Combo: TD 键也参与组合 ──
    {"name": "combo_as_left", "desc": "combo a+s→Left (a和s都是TD键!)",
     "steps": [{"dn": "a"}, {"dn": "s"}, {"up": "s"}, {"up": "a"}, {"wait": 500}]},

    # ── 层内 combo ──
    {"name": "combo_fn1_hj_home", "desc": "有fn1时 h+j→Home",
     "steps": [{"dn": "space"}, {"wait": 300}, {"dn": "h"}, {"dn": "j"}, {"up": "j"}, {"up": "h"}, {"up": "space"}, {"wait": 500}]},

    # ── 同名键跨层 ──
    {"name": "samekey_a_fn1", "desc": "同名键a: 在fn1内tap→A(大写)",
     "steps": [{"dn": "space"}, {"wait": 300}, {"dn": "a"}, {"up": "a"}, {"up": "space"}, {"wait": 500}]},
    {"name": "samekey_s_fn2", "desc": "同名键s: 在fn2内tap→hello文本",
     "steps": [{"dn": "s"}, {"wait": 300}, {"dn": "k"}, {"up": "k"}, {"up": "s"}, {"wait": 500}]},
    {"name": "samekey_j_fn3", "desc": "同名键j: 在fn3内tap→{Up}",
     "steps": [{"dn": "j"}, {"wait": 300}, {"dn": "j"}, {"up": "j"}, {"up": "j"}, {"wait": 500}]},

    # ── Defer: hold 时间内按另一键 ──
    {"name": "defer_space_h", "desc": "Space dn后立即按h→defer",
     "steps": [{"dn": "space"}, {"dn": "h"}, {"up": "h"}, {"up": "space"}, {"wait": 500}]},
    {"name": "defer_l_k", "desc": "l(hold→fn1) dn后立即按k→defer",
     "steps": [{"dn": "l"}, {"dn": "k"}, {"up": "k"}, {"up": "l"}, {"wait": 500}]},

    # ── 多层堆叠：多个 hold 键 ──
    {"name": "stack_space_s", "desc": "Space hold→fn1 + s hold→fn2 堆叠",
     "steps": [{"dn": "space"}, {"wait": 300}, {"dn": "s"}, {"wait": 300}, {"up": "s"}, {"up": "space"}, {"wait": 500}]},
    {"name": "stack_three_layers", "desc": "三层堆叠: Space→fn1, s→fn2, j→fn3",
     "steps": [{"dn": "space"}, {"wait": 300}, {"dn": "s"}, {"wait": 300},
               {"dn": "j"}, {"wait": 300}, {"up": "j"}, {"up": "s"}, {"up": "space"}, {"wait": 500}]},

    # ── fn4: 宏和修饰键 ──
    {"name": "fn4_run", "desc": "fn4层 1→RUN:notepad.exe",
     "steps": [{"dn": "j"}, {"wait": 300}, {"dn": "1"}, {"up": "1"}, {"up": "j"}, {"wait": 500}]},
    {"name": "fn4_macro", "desc": "fn4层 2→^c 宏",
     "steps": [{"dn": "j"}, {"wait": 300}, {"dn": "2"}, {"up": "2"}, {"up": "j"}, {"wait": 500}]},

    # ── {bnX} 拦截测试 ──
    {"name": "bn_intercept", "desc": "bn键 hold→{bn1} 应拦截不输出, fn键 hold→{fn1} 正常激活",
     "steps": [{"dn": "space"}, {"wait": 300}, {"dn": "a"}, {"up": "a"}, {"up": "space"}, {"wait": 500}]},

    # ── holding auto-repeat: 修饰键 hold ──
    {"name": "td_hold_repeat_shift", "desc": "z长按→{shift}, auto-repeat 重复输出 shift",
     "steps": [{"dn": "z"}, {"wait": 500}, {"dn": "z"}, {"up": "z"}, {"wait": 500}]},

    # ── defer: space+z 快速序列，z 被 defer → z 抬起打断 space doubletap ──
    {"name": "defer_space_z", "desc": "space(dt)+z快速: z被defer, z抬起打断space doubletap等",
     "steps": [{"dn": "space"}, {"dn": "z"}, {"up": "space"}, {"up": "z"}, {"wait": 500}]},
]
