/// {tnX} toggle layer — 场景测试
///
/// 覆盖设计决策文档 docs/AnyKey_ToggleLayer_tnX_设计决策.md 的 D1~D8：
///   - tn 基本切换（keyup 静默不反激活）
///   - tn1 被 fn2 临时覆盖后松开回 tn1
///   - tn0 重置 layer_stack
///   - 重复输入 tn 不多次压栈（D8 幂等，activated_by 检查）
///   - 先 fn1 再 tn1 再都松开 → 留在 fn1（D8 修复的核心 bug 场景）
///   - combo 输出 tn 正确切换且不被反激活
///   - combo 链式（a+s=tn1、s+d=fn2）后 tn 不被反激活
///
/// 层键定义严格链式（用户确认）：切层后当前键盘完全属于当前层，
/// 故「在某层按下另一个层键」时，该层键必须定义在该层（否则退化为 tap）。

use anykey_engine::state::*;
use anykey_engine::config::*;
use std::collections::HashMap;

fn base_cfg() -> Config {
    Config {
        combo_time: 200,
        combo_map: vec![],
        layers: Layers {
            layer_maps: HashMap::new(),
            hold_term: 200,
            double_tap_term: 250,
            double_hold_term: 150,
        },
        leader: LeaderConfig::default(),
        subscribed_devices: vec![],
        app_aware: Default::default(),
        per_device: false,
        devices: HashMap::new(),
    }
}

fn new_state(cfg: Config) -> PipelineState {
    PipelineState::new(cfg)
}

fn count_layer_on(s: &PipelineState, name: &str) -> usize {
    s.emit_log.iter().filter(|e| matches!(e, EmitEvent::LayerOn(n, _) if n == name)).count()
}

fn count_layer_off(s: &PipelineState, name: &str) -> usize {
    s.emit_log.iter().filter(|e| matches!(e, EmitEvent::LayerOff(n, _) if n == name)).count()
}

/// 场景 1：tn 基本切换——hold 触发切层，keyup 静默不反激活。
#[test]
fn test_tn_basic_toggle_persists() {
    let mut cfg = base_cfg();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{tn1}".into(), ht: "200".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("f");
    assert_eq!(s.current_layer(), "base", "hold_term 未到，不应激活");
    s.advance_ms(250); // 越过 hold_term → hold 触发 tn1
    assert_eq!(s.current_layer(), "fn1", "hold 应激活 fn1");
    assert_eq!(count_layer_on(&s, "fn1"), 1);

    s.key_up("f"); // tn keyup 静默，不反激活
    assert_eq!(s.current_layer(), "fn1", "tn 键抬起后应保持 fn1");
    assert_eq!(count_layer_off(&s, "fn1"), 0, "tn 键不应反激活 fn1");
    assert_eq!(s.state.layers.layer_stack.len(), 1, "fn1 应保留在栈中");
}

/// 场景 2：tn1 之后被 fn2 临时覆盖，松开 fn2 回 tn1 指向的层。
/// 严格链式：fn2 键 g 定义在 fn1 层（切到 fn1 后当前键盘即 fn1 层）。
#[test]
fn test_tn_covered_by_fn_then_revert() {
    let mut cfg = base_cfg();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{tn1}".into(), ht: "200".into(), ..Default::default()
    });
    cfg.layers.layer_maps.entry("fn1".into()).or_default().insert("g".into(), KeyEntry {
        tap: "{g}".into(), hold: "{fn2}".into(), ht: "200".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    // tn1 切换
    s.key_down("f"); s.advance_ms(250); s.key_up("f");
    assert_eq!(s.current_layer(), "fn1");

    // fn2 临时覆盖（g 在 fn1 层定义 hold={fn2}）
    s.key_down("g"); s.advance_ms(250);
    assert_eq!(s.current_layer(), "fn2", "fn2 应覆盖 fn1");
    assert_eq!(count_layer_on(&s, "fn2"), 1);

    // 松开 fn2 → 反激活 fn2，回到 tn1 指向的 fn1
    s.key_up("g");
    assert_eq!(s.current_layer(), "fn1", "松开 fn2 应回到 tn1 指向的层");
    assert_eq!(count_layer_off(&s, "fn2"), 1, "fn2 应反激活");
    assert_eq!(count_layer_off(&s, "fn1"), 0, "tn1 不应被反激活");
}

/// 场景 3：tn0 重置 layer_stack（清空所有层回 base）。
/// 严格链式：tn0 键 h 定义在 fn1 层。
#[test]
fn test_tn0_resets_layer_stack() {
    let mut cfg = base_cfg();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{tn1}".into(), ht: "200".into(), ..Default::default()
    });
    cfg.layers.layer_maps.entry("fn1".into()).or_default().insert("h".into(), KeyEntry {
        tap: "{h}".into(), hold: "{tn0}".into(), ht: "200".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("f"); s.advance_ms(250); s.key_up("f");
    assert_eq!(s.current_layer(), "fn1");
    assert_eq!(s.state.layers.layer_stack.len(), 1);

    s.key_down("h"); s.advance_ms(250); // tn0 清空
    assert_eq!(s.current_layer(), "base", "tn0 应清空 layer_stack 回到 base");
    assert_eq!(s.state.layers.layer_stack.len(), 0, "tn0 后栈应为空");
    assert_eq!(count_layer_off(&s, "fn1"), 1, "tn0 清空应对 fn1 发 LayerOff");
}

/// 场景 4：重复输入 tn 不会多次压栈（D8 幂等，activated_by 检查）。
/// 严格链式：f 在 fn1 层也定义 hold={tn1}，第二次按下才真正走 hold 路径（而非退化 tap）。
#[test]
fn test_tn_repeat_no_restack() {
    let mut cfg = base_cfg();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{tn1}".into(), ht: "200".into(), ..Default::default()
    });
    cfg.layers.layer_maps.entry("fn1".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{tn1}".into(), ht: "200".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    // 第一次切换
    s.key_down("f"); s.advance_ms(250); s.key_up("f");
    assert_eq!(s.state.layers.layer_stack.len(), 1);
    assert_eq!(count_layer_on(&s, "fn1"), 1);

    // 第二次切换（f 在 fn1 层有定义 → 走 hold 路径；activated_by=f 已在栈 → 幂等不压栈）
    s.key_down("f"); s.advance_ms(250); s.key_up("f");
    assert_eq!(s.state.layers.layer_stack.len(), 1, "重复输入 tn 不应多次压栈");
    assert_eq!(count_layer_on(&s, "fn1"), 1, "重复输入 tn 不应重复发 LayerOn");
    assert_eq!(s.current_layer(), "fn1");
}

/// 场景 7（D8 修复的核心）：先 fn1（临时）再 tn1（toggle），都松开后应留在 fn1，
/// 而不是回到 base。修复前 D8 用「name == current_layer」会跳过 tn1 压栈，
/// 导致 fn1 键松开后 fn1 完全消失。
/// 严格链式：f（tn1）在 fn1 层也定义 hold={tn1}。
#[test]
fn test_tn_after_fn_both_release_stays() {
    let mut cfg = base_cfg();
    // base 层：g 临时层键（hold={fn1}），f toggle 层键（hold={tn1}）
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("g".into(), KeyEntry {
        tap: "{g}".into(), hold: "{fn1}".into(), ht: "200".into(), ..Default::default()
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{tn1}".into(), ht: "200".into(), ..Default::default()
    });
    // fn1 层：f 也定义 hold={tn1}（严格链式，切到 fn1 后 f 仍是 toggle 键）
    cfg.layers.layer_maps.entry("fn1".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{tn1}".into(), ht: "200".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    // g 先激活 fn1（临时，压栈 entry(g, fn1)）
    s.key_down("g"); s.advance_ms(250);
    assert_eq!(s.current_layer(), "fn1");

    // f 再触发 tn1（f 在 fn1 层有定义 → 走 hold；activated_by=f 不在栈 → 压栈 entry(f, fn1)）
    s.key_down("f"); s.advance_ms(250);
    assert_eq!(s.current_layer(), "fn1");
    assert_eq!(s.state.layers.layer_stack.len(), 2, "g 和 f 各压一个 entry");

    // 松开 g：反激活 g 的 fn1 entry，f 的 tn1 entry 保留
    s.key_up("g");
    assert_eq!(s.current_layer(), "fn1", "g 松开后 f 的 tn1 entry 应保留");
    assert_eq!(s.state.layers.layer_stack.len(), 1);

    // 松开 f：tn 静默，不反激活
    s.key_up("f");
    assert_eq!(s.current_layer(), "fn1", "全部松开后应留在 fn1（tn1 固定），而非回 base");
    assert_eq!(s.state.layers.layer_stack.len(), 1);
    // g 的临时层反激活发 1 次 LayerOff（g 松开时），f 的 tn1 静默不再发 → 总计 1 次
    assert_eq!(count_layer_off(&s, "fn1"), 1, "仅 g 的临时层反激活发 LayerOff，f 的 tn1 不反激活");
    assert_eq!(s.state.layers.layer_stack[0].resolved_output, "{tn1}",
        "栈中保留的应是 f 的 tn1 entry（而非 g 的 fn1 entry）");
}

/// 场景 5：combo 输出 tn 能正确切换，且释放后不被反激活。
#[test]
fn test_combo_tn_toggle_not_deactivated() {
    let mut cfg = base_cfg();
    cfg.combo_map.push(ComboRow {
        key1: "a".into(), key2: "s".into(),
        output: "{tn1}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry { tap: "{a}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("s".into(), KeyEntry { tap: "{s}".into(), ..Default::default() });
    let mut s = new_state(cfg);

    s.key_down("a");
    s.key_down("s"); // combo 匹配 → tn1 激活
    assert_eq!(s.current_layer(), "fn1", "combo 应激活 fn1");
    assert_eq!(count_layer_on(&s, "fn1"), 1);

    s.key_up("s");
    s.key_up("a"); // 释放 → tn 不反激活
    assert_eq!(s.current_layer(), "fn1", "combo tn 释放后应保持 fn1");
    assert_eq!(count_layer_off(&s, "fn1"), 0, "combo tn 不应被反激活");
}

/// 场景 6：combo 链式——a+s=tn1、s+d=fn2。
/// 操作 a↓ s↓ s↑ s↓ d↓ a↑ s↑ d↑，期望：激活 fn1（tn，不反激活）、激活 fn2、反激活 fn2。
#[test]
fn test_combo_chain_tn_then_fn() {
    let mut cfg = base_cfg();
    cfg.combo_map.push(ComboRow {
        key1: "a".into(), key2: "s".into(),
        output: "{tn1}".into(), layer: "base".into(),
    });
    cfg.combo_map.push(ComboRow {
        key1: "s".into(), key2: "d".into(),
        output: "{fn2}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry { tap: "{a}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("s".into(), KeyEntry { tap: "{s}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("d".into(), KeyEntry { tap: "{d}".into(), ..Default::default() });
    let mut s = new_state(cfg);

    s.key_down("a");
    s.key_down("s"); // a+s → tn1，激活 fn1
    assert_eq!(s.current_layer(), "fn1");

    s.key_up("s");
    s.key_down("s");
    s.key_down("d"); // s+d → fn2，激活 fn2（链式）
    assert_eq!(s.current_layer(), "fn2", "s+d 应激活 fn2");

    s.key_up("a");
    s.key_up("s");
    s.key_up("d"); // fn2 反激活

    assert_eq!(count_layer_on(&s, "fn1"), 1, "应激活 fn1");
    assert_eq!(count_layer_on(&s, "fn2"), 1, "应激活 fn2");
    assert_eq!(count_layer_off(&s, "fn2"), 1, "应反激活 fn2");
    assert_eq!(count_layer_off(&s, "fn1"), 0, "tn1 不应被反激活");
    assert_eq!(s.current_layer(), "fn1", "链式结束后应回到 tn1 指向的 fn1");
}

/// 场景 8：tn 键 tap 有值（D2 零约束）——短按输出 tap，长按 toggle，两者不互相干扰。
#[test]
fn test_tn_tap_field_has_value() {
    let mut cfg = base_cfg();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{x}".into(), hold: "{tn1}".into(), ht: "200".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    // 短按：tap 输出 x，不进层
    s.key_down("f");
    s.key_up("f");
    assert_eq!(s.current_layer(), "base", "短按 tap 不应激活层");
    assert_eq!(count_layer_on(&s, "fn1"), 0);
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) | EmitEvent::Tap(k, _) if k == "x")),
        "短按应输出 tap=x");

    // 长按：toggle 切层
    s.key_down("f");
    s.advance_ms(250);
    s.key_up("f");
    assert_eq!(s.current_layer(), "fn1", "长按应 toggle 切到 fn1");
    assert_eq!(count_layer_on(&s, "fn1"), 1);
}

/// 场景 9：同名层不误伤——tn1 键和 fn1 键都指向 fn1，fn1 键松开时不能误反激活 tn1 的 entry。
/// 验证 (activated_by, name) 精确出栈。
#[test]
fn test_tn_fn_same_layer_no_confusion() {
    let mut cfg = base_cfg();
    // base 层：f toggle（hold={tn1}），g 临时层键（hold={fn1}）
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{tn1}".into(), ht: "200".into(), ..Default::default()
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("g".into(), KeyEntry {
        tap: "{g}".into(), hold: "{fn1}".into(), ht: "200".into(), ..Default::default()
    });
    // fn1 层：g 也定义 hold={fn1}（严格链式，切到 fn1 后 g 仍是临时层键）
    cfg.layers.layer_maps.entry("fn1".into()).or_default().insert("g".into(), KeyEntry {
        tap: "{g}".into(), hold: "{fn1}".into(), ht: "200".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    // f 先 toggle fn1（压栈 entry(f, fn1, {tn1})）
    s.key_down("f"); s.advance_ms(250); s.key_up("f");
    assert_eq!(s.current_layer(), "fn1");
    assert_eq!(s.state.layers.layer_stack.len(), 1);

    // g 临时激活 fn1（g 在 fn1 层有定义 → 压栈 entry(g, fn1, {fn1})）
    s.key_down("g"); s.advance_ms(250);
    assert_eq!(s.state.layers.layer_stack.len(), 2, "g 应压第二个 entry");

    // g 松开 → 精确反激活 g 的 entry，f 的 tn1 entry 保留
    s.key_up("g");
    assert_eq!(s.state.layers.layer_stack.len(), 1, "g 松开后应只剩 f 的 entry");
    assert_eq!(s.current_layer(), "fn1");
    assert_eq!(s.state.layers.layer_stack[0].resolved_output, "{tn1}",
        "栈中保留的应是 f 的 tn1 entry（而非 g 的 fn1 entry）");
}

/// 场景 10：tn0 在栈空时触发——不 panic、不发多余 LayerOff。
#[test]
fn test_tn0_empty_stack_no_panic() {
    let mut cfg = base_cfg();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("h".into(), KeyEntry {
        tap: "{h}".into(), hold: "{tn0}".into(), ht: "200".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("h");
    s.advance_ms(250); // tn0 清空（栈本就空）
    s.key_up("h");
    assert_eq!(s.current_layer(), "base");
    assert_eq!(s.state.layers.layer_stack.len(), 0);
    assert_eq!(count_layer_off(&s, "fn1"), 0, "栈空 tn0 不应发 LayerOff");
}

/// 场景 11：combo 输出 {tn0} 清空 layer_stack。
#[test]
fn test_combo_tn0_resets() {
    let mut cfg = base_cfg();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{tn1}".into(), ht: "200".into(), ..Default::default()
    });
    cfg.combo_map.push(ComboRow {
        key1: "a".into(), key2: "s".into(),
        output: "{tn0}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry { tap: "{a}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("s".into(), KeyEntry { tap: "{s}".into(), ..Default::default() });
    let mut s = new_state(cfg);

    // 先 tn1 切换 fn1
    s.key_down("f"); s.advance_ms(250); s.key_up("f");
    assert_eq!(s.current_layer(), "fn1");
    assert_eq!(s.state.layers.layer_stack.len(), 1);

    // combo 输出 tn0 → 清空
    s.key_down("a");
    s.key_down("s");
    assert_eq!(s.current_layer(), "base", "combo tn0 应清空 layer_stack");
    assert_eq!(s.state.layers.layer_stack.len(), 0);
    assert_eq!(count_layer_off(&s, "fn1"), 1, "combo tn0 应对 fn1 发 LayerOff");
}
