"""生成堆叠/修饰键 新场景 golden（自包含 config，expect 首轮留空以观察引擎真实输出）。

运行: cd AnyKey && python scripts/_gen_stacking_scenarios.py
然后跑 cargo test --test scenario_test 观察真实 emit_log。
"""
import os, json

PROJECT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCEN = os.path.join(PROJECT, "tests", "scenarios")


def kc(tap, hold="", ht="", dt="", dtt="", dh="", dht=""):
    return {"tap": tap, "hold": hold, "ht": ht, "dt": dt,
            "dtt": dtt, "dh": dh, "dht": dht}


def layer_def(name, key_map=None):
    return {"name": name, "blockHold": False, "keyMap": key_map or {}}


def base_cfg(base_layer, layer_defs, combo_map):
    return {
        "comboTime": 200,
        "comboMap": combo_map,
        "layers": {
            "enabled": True,
            "baseLayer": base_layer,
            "switchKeys": [],
            "layerDefs": layer_defs,
        },
        "tapDance": {
            "enabled": True,
            "holdTerm": 200,
            "doubleTapTerm": 250,
            "doubleHoldTerm": 200,
            "keys": {},
        },
        "flowControl": {"comboToLayer": False, "comboToTapDance": False},
        "subscribed_devices": [],
    }


EMPTY_LAYERS = [layer_def("fn1"), layer_def("fn2"), layer_def("fn3"), layer_def("fn4")]

# ── 修饰键组 config（z=shift, 6/x 普通）──
mod_base = {
    "z": kc("{z}", hold="{shift}", ht="200"),
    "6": kc("{6}"),
    "x": kc("{x}"),
}
mod_cfg = base_cfg(mod_base, EMPTY_LAYERS, [])

# ── B1: Modifier + layer + 普通键（fn1.c -> {x}）──
b1_base = {
    "m": kc("{m}", hold="{shift}", ht="200"),
    "f": kc("{f}", hold="{fn1}", ht="200"),
    "c": kc("{c}"),
}
b1_cfg = base_cfg(b1_base, [layer_def("fn1", {"c": kc("{x}")}),
                            layer_def("fn2"), layer_def("fn3"), layer_def("fn4")], [])

# ── B2: layer + Modifier + combo（fn1 层 h+j -> {Home}）──
b2_base = {
    "f": kc("{f}", hold="{fn1}", ht="200"),
    "s": kc("{s}", hold="{shift}", ht="200"),
    "h": kc("{h}"),
    "j": kc("{j}"),
}
b2_combo = [{"key1": "h", "key2": "j", "output": "{Home}", "layer": "fn1"}]
b2_cfg = base_cfg(b2_base, [layer_def("fn1"), layer_def("fn2"),
                            layer_def("fn3"), layer_def("fn4")], b2_combo)

# ── B3: 多 layer + 多 Modifier（fn1/fn2 层栈 + ctrl+alt；fn2.k -> {y}）──
b3_base = {
    "f1": kc("{f1}", hold="{fn1}", ht="200"),
    "f2": kc("{f2}", hold="{fn2}", ht="200"),
    "q": kc("{q}", hold="{ctrl}", ht="200"),
    "w": kc("{w}", hold="{alt}", ht="200"),
    "k": kc("{k}"),
}
b3_cfg = base_cfg(b3_base, [layer_def("fn1"),
                            layer_def("fn2", {"k": kc("{y}")}),
                            layer_def("fn3"), layer_def("fn4")], [])


# 场景定义: (name, desc, cfg, steps)
SCENARIOS = [
    # 修饰键 10ms 重叠 / keyup 先到
    ("mod_10ms_zup_first", "z hold=shift, 6 普通; z↓6↓ z↑6↑ (10ms) — z的keyup先到, flush 6; 期待 z6 无shift",
     mod_cfg, [{"dn": "z"}, {"wait": 10}, {"dn": "6"}, {"wait": 10}, {"up": "z"}, {"wait": 10}, {"up": "6"}]),
    ("mod_10ms_6up_first", "z hold=shift, 6 普通; z↓6↓ 6↑z↑ (10ms) — 被defer的6的keyup先到; 关键: defer后 keyup先到能否正常",
     mod_cfg, [{"dn": "z"}, {"wait": 10}, {"dn": "6"}, {"wait": 10}, {"up": "6"}, {"wait": 10}, {"up": "z"}]),
    # 慢速 modifier 打断（hold 真正触发 shift）
    ("mod_slow_defer", "z hold=shift; z↓ (100) 6↓ (250, defer后shift触发) 6↑ z↑ — 6 被defer后随shift生效",
     mod_cfg, [{"dn": "z"}, {"wait": 100}, {"dn": "6"}, {"wait": 250}, {"up": "6"}, {"up": "z"}]),
    ("mod_slow_active", "z hold=shift; z↓ (300, shift已激活) 6↓ 6↑ z↑ — 打断键在shift激活后直接生效",
     mod_cfg, [{"dn": "z"}, {"wait": 300}, {"dn": "6"}, {"up": "6"}, {"up": "z"}]),
    # 非 TD 键双按 -> 两个 tap（无 doubletap 行为）
    ("nondt_doubletap", "x 仅 tap={x} (无 dt/dh); x↓x↑x↓x↑ 快速双按 — 期待两个独立 tap, 不冒出 doubletap/null",
     mod_cfg, [{"dn": "x"}, {"up": "x"}, {"dn": "x"}, {"up": "x"}]),
    # B1: Modifier + layer + 普通键 快速版
    ("stack_mod_layer_key_fast", "m=shift, f=fn1, c普通(fn1.c->{x}); m↓(250) f↓(250) c↓ c↑ f↑ m↑ — 普通键keyup先到, 测试keyup重叠",
     b1_cfg, [{"dn": "m"}, {"wait": 250}, {"dn": "f"}, {"wait": 250}, {"dn": "c"}, {"up": "c"}, {"up": "f"}, {"up": "m"}]),
    # B2: layer + Modifier + combo 快速版
    ("stack_layer_mod_combo_fast", "f=fn1, s=shift, h+j combo(fn1->{Home}); f↓(250) s↓(250) h↓ j↓ j↑ h↑ s↑ f↑ — combo keyup先到",
     b2_cfg, [{"dn": "f"}, {"wait": 250}, {"dn": "s"}, {"wait": 250}, {"dn": "h"}, {"dn": "j"}, {"up": "j"}, {"up": "h"}, {"up": "s"}, {"up": "f"}]),
    # B3: 多 layer + 多 Modifier 快速版
    ("stack_multilayer_multimod_fast", "f1=fn1,f2=fn2,q=ctrl,w=alt,k普通(fn2.k->{y}); 全按下后 k↓k↑ 先到, 再释放mods/layers",
     b3_cfg, [{"dn": "f1"}, {"wait": 250}, {"dn": "f2"}, {"wait": 250}, {"dn": "q"}, {"wait": 250},
              {"dn": "w"}, {"wait": 250}, {"dn": "k"}, {"up": "k"}, {"up": "w"}, {"up": "q"}, {"up": "f2"}, {"up": "f1"}]),
]


def main():
    for name, desc, cfg, steps in SCENARIOS:
        built = {
            "name": name,
            "ahi_incompatible": True,
            "desc": desc,
            "config": cfg,
            "steps": steps,
            "expect": [],  # 首轮留空, 观察引擎真实输出
        }
        path = os.path.join(SCEN, f"{name}.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(built, f, indent=2, ensure_ascii=False)
        print("wrote", name)


if __name__ == "__main__":
    main()
