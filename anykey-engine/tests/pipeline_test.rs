/// AnyKey engine - Pipeline behavior tests

use anykey_engine::state::*;
use anykey_engine::config::*;
use anykey_engine::util::*;
use std::collections::HashMap;

fn test_config() -> Config {
    Config {
        combo_time: 200,
        combo_map: vec![],
        layers: Layers {
            
            layer_maps: HashMap::new(),
            hold_term: 150,
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

#[test]
fn test_empty_config_doesnt_panic() {
    let s = new_state(test_config());
    assert_eq!(s.emit_log.len(), 0);
    assert_eq!(s.current_layer(), "base");
}

#[test]
fn test_simple_key_down_up() {
    let mut cfg = test_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("a");
    assert!(s.emit_log.len() >= 1, "a should emit on key_down");
    match s.emit_log.last().unwrap() {
        EmitEvent::Down(k, _) => assert_eq!(k, "a", "expected Down a"),
        other => panic!("Expected Down event, got {:?}", other),
    }

    s.key_up("a");
    match s.emit_log.last().unwrap() {
        EmitEvent::Up(k, _) => assert_eq!(k, "a"),
        EmitEvent::Tap(k, _) => assert_eq!(k, "a"),
        other => panic!("Expected Up/Tap event, got {:?}", other),
    }
}

#[test]
fn test_emit_log_events() {
    let mut cfg = test_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("a");
    assert!(!s.emit_log.is_empty());
    match s.emit_log.last().unwrap() {
        EmitEvent::Down(k, _) => assert_eq!(k, "a"),
        other => panic!("Expected Down, got {:?}", other),
    }

    s.key_up("a");
    let last = s.emit_log.last().unwrap();
    assert!(matches!(last, EmitEvent::Up(_, _) | EmitEvent::Tap(_, _)));
}

#[test]
fn test_key_down_mapping_tracks_emitted_keys() {
    let mut cfg = test_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("a");
    assert!(s.emitted("a").is_some());
    // emitted 现记录完整 {X} 形式（与 phase7_final 的 logical_key 一致）
    assert_eq!(s.emitted("a").unwrap(), "{a}");

    s.key_up("a");
    assert!(s.emitted("a").is_none());
}

/// 方案3（层键与 emitted 解耦）：层键 hold 激活后，输出写入 target_layer 字段、emitted 留空；
/// 释放时由 release_key 先读 target_layer 完成反激活，并清空。这是 phase8_final 解耦后的核心契约。
#[test]
fn test_target_layer_records_layer_key() {
    let mut cfg = test_config();
    cfg.layers.hold_term = 200;
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("space".into(), KeyEntry {
        tap: "{Space}".into(), hold: "{fn1}".into(), ht: "200".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("space");
    // 推进越过 hold_term 触发 hold → fn1 激活
    s.advance_ms(250);

    // 层键写入 target_layer、emitted 留空，且层已激活
    assert!(s.emitted("space").is_none(), "层键不应写入 emitted");
    assert_eq!(s.target_layer("space").unwrap(), "fn1");
    assert_eq!(s.current_layer(), "fn1");
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::LayerOn(n, _) if n == "fn1")));

    s.key_up("space");

    // 释放后 target_layer 清空、层反激活
    assert!(s.target_layer("space").is_none());
    assert!(s.emitted("space").is_none());
    assert_eq!(s.current_layer(), "base");
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::LayerOff(n, _) if n == "fn1")));
}

/// 回归：层键 hold 期间 auto-repeat（重复 key_down）不得把 target_layer 覆盖掉，
/// 否则抬起时层不反激活。方案3 下：层键不进 emitted（即使 auto-repeat 把物理 tap 写进 emitted，
/// 也绝不能写成层键），层反激活完全由 target_layer 驱动，故仍能正确反激活。
#[test]
fn test_layer_key_autorepeat_keeps_target_layer() {
    let mut cfg = test_config();
    cfg.layers.hold_term = 200;
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("space".into(), KeyEntry {
        tap: "{Space}".into(), hold: "{fn1}".into(), ht: "200".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("space");
    s.advance_ms(250);             // hold 触发 → fn1 激活，target_layer=fn1
    assert!(s.emitted("space").is_none(), "层键不应写入 emitted");
    assert_eq!(s.target_layer("space").unwrap(), "fn1");

    // 模拟 OS auto-repeat：按住期间多次重复 key_down
    for _ in 0..5 {
        s.key_down("space");
        // emitted 可能被 auto-repeat 写成物理 tap（{Space}，方案3 不动其余逻辑），
        // 但绝不能写成层键（否则旧 bug 会误把层键当 emitted 反激活）。
        if let Some(em) = s.emitted("space") {
            assert!(!is_layer_key(em), "auto-repeat 不应把层键写入 emitted");
        }
        assert_eq!(s.target_layer("space").unwrap(), "fn1", "auto-repeat 应保持 target_layer");
    }
    assert_eq!(s.current_layer(), "fn1", "层应保持激活");

    s.key_up("space");
    assert_eq!(s.current_layer(), "base", "抬起后层必须反激活（方案3 核心修复）");
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::LayerOff(n, _) if n == "fn1")));
    assert!(s.target_layer("space").is_none());
}

#[test]
fn test_layer_stack_activation_deactivation() {
    let mut s = new_state(test_config());
    assert_eq!(s.current_layer(), "base");

    s.activate_layer("fn1", "", "", "");
    assert_eq!(s.current_layer(), "fn1");

    s.activate_layer("fn1", "", "", "");
    assert_eq!(s.current_layer(), "fn1");

    s.deactivate_layer("", "fn1");
    assert_eq!(s.current_layer(), "fn1");

    s.deactivate_layer("", "fn1");
    assert_eq!(s.current_layer(), "base");

    // 去计数器后：每次压栈/出栈都发事件（无 0↔1 跃迁去重，嵌套激活同层连续出现多个事件属正常）
    assert_eq!(s.emit_log, vec![
        EmitEvent::LayerOn("fn1".to_string(), 1),
        EmitEvent::LayerOn("fn1".to_string(), 1),
        EmitEvent::LayerOff("fn1".to_string(), 1),
        EmitEvent::LayerOff("fn1".to_string(), 1),
    ]);
}

#[test]
fn test_nested_layer_stack() {
    let mut s = new_state(test_config());
    s.activate_layer("fn1", "", "", "");
    s.activate_layer("fn2", "", "", "");
    assert_eq!(s.current_layer(), "fn2");
    s.deactivate_layer("", "fn2");
    assert_eq!(s.current_layer(), "fn1");

    // 层事件应按激活/反激活跃迁顺序进入 emit_log（每次操作各发一次）
    assert_eq!(s.emit_log, vec![
        EmitEvent::LayerOn("fn1".to_string(), 1),
        EmitEvent::LayerOn("fn2".to_string(), 1),
        EmitEvent::LayerOff("fn2".to_string(), 1),
    ]);
}

#[test]
fn test_td_waiting_on_keydown() {
    let mut cfg = test_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(),
        hold: "{fn1}".into(),
        ht: "150".into(),
        ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("a");
    assert!(s.state.keys.key_states.get("a").map_or(false, |__ks| __ks.td.is_some()), "a should have td_state");
    assert_eq!(s.state.keys.key_states["a"].td.as_ref().unwrap().kind, TdKind::Waiting, "a should be Waiting");
}

#[test]
fn test_td_tap_on_keyup() {
    let mut cfg = test_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(),
        hold: "{fn1}".into(),
        ht: "200".into(),
        ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("a");
    assert_eq!(s.state.keys.key_states["a"].td.as_ref().unwrap().kind, TdKind::Waiting);

    s.key_up("a");
    // After key_up, td should be finished
    assert!(s.state.keys.key_states.get("a").and_then(|__ks| __ks.td.as_ref()).map_or(true, |s| s.kind == TdKind::Finished),
        "tap should finish td after keyup");
    assert!(s.emit_log.len() > 0, "should emit after tap");
}

#[test]
fn test_combo_waiting_state() {
    let mut cfg = test_config();
    cfg.combo_map.push(ComboRow {
        key1: "a".into(), key2: "s".into(),
        output: "{d}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(), ..Default::default()
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("s".into(), KeyEntry {
        tap: "{s}".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("a");
    assert!(s.has_combo("a"));
    assert_eq!(s.state.keys.key_states["a"].combo.as_ref().unwrap().state, ComboKind::Seeking);

    s.key_down("s");
    assert!(s.emit_log.len() > 0, "combo should produce output");
}

#[test]
fn test_combo_key_does_not_emit_while_waiting() {
    let mut cfg = test_config();
    cfg.combo_map.push(ComboRow {
        key1: "q".into(), key2: "w".into(),
        output: "{enter}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("q".into(), KeyEntry {
        tap: "{q}".into(), ..Default::default()
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("w".into(), KeyEntry {
        tap: "{w}".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("q");
    assert!(s.has_combo("q"));
    // q is waiting for partner - should not have emitted yet
    assert_eq!(s.emit_log.len(), 0, "waiting combo key should not emit");
}

#[test]
fn test_combo_interrupt_by_third_key() {
    let mut cfg = test_config();
    cfg.combo_map.push(ComboRow {
        key1: "a".into(), key2: "s".into(),
        output: "{d}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(), ..Default::default()
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("s".into(), KeyEntry {
        tap: "{s}".into(), ..Default::default()
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("d".into(), KeyEntry {
        tap: "{d}".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("a");
    assert!(s.has_combo("a"));

    s.key_down("d"); // non-partner → should interrupt 'a'
    s.key_up("a"); // physically release interrupted key → combo cleared on keyup
    assert!(!s.has_combo("a"));
}

#[test]
fn test_flow_control_combo_to_layer_false() {
    let mut cfg = test_config();    cfg.combo_map.push(ComboRow {
        key1: "a".into(), key2: "s".into(),
        output: "{d}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(), ..Default::default()
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("s".into(), KeyEntry {
        tap: "{s}".into(), ..Default::default()
    });
    let mut s = new_state(cfg);

    s.key_down("a");
    s.key_down("s");
    assert!(s.emit_log.len() > 0, "combo should match with flow_control");
}

#[test]
fn test_layer_activation_on_tap() {
    let mut cfg = test_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(),
        hold: "{fn1}".into(),
        ht: "100".into(),
        ..Default::default()
    });
    cfg.layers.layer_maps.insert("fn1".into(), {
            let mut m = HashMap::new();
            m.insert("j".into(), KeyEntry {
                tap: "{Left}".into(), ..Default::default()
            });
            m
        });
    let mut s = new_state(cfg);

    assert_eq!(s.current_layer(), "base");
    s.key_down("a");
    assert_eq!(s.state.keys.key_states["a"].td.as_ref().unwrap().kind, TdKind::Waiting);
    s.key_up("a"); // tap, no layer activation
    assert_eq!(s.current_layer(), "base", "tap should not activate layer");
}

// ═══════════════════════════════════════════
// Mouse key tests
// ═══════════════════════════════════════════

#[test]
fn test_norm_key_mouse_mapping() {
    // 鼠标键不再映射为 lbutton/rbutton/mbutton，保持自身统一名（小写）
    assert_eq!(norm_key("mouseleft"), "mouseleft");
    assert_eq!(norm_key("mouseright"), "mouseright");
    assert_eq!(norm_key("mousemiddle"), "mousemiddle");
    assert_eq!(norm_key("mouseside1"), "mouseside1");
    assert_eq!(norm_key("mouseside2"), "mouseside2");
    // case insensitive → 同一个小写结果
    assert_eq!(norm_key("MouseLeft"), "mouseleft");
    assert_eq!(norm_key("MOUSESIDE1"), "mouseside1");
    // Wheel 键不受影响
    assert_eq!(norm_key("WheelUp"), "wheelup");
    assert_eq!(norm_key("wheeldown"), "wheeldown");
}

#[test]
fn test_mouse_button_td_hold() {
    let mut cfg = test_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("mouseside1".into(), KeyEntry {
        tap: "{mouseside1}".into(),
        hold: "{pgup}".into(),
        ht: "200".into(),
        ..Default::default()
    });
    let mut s = new_state(cfg);
    s.debug_enabled = true;

    s.key_down("mouseside1");
    assert_eq!(s.state.keys.key_states["mouseside1"].td.as_ref().unwrap().kind, TdKind::Waiting);
    s.advance_ms(250);
    assert!(!s.emit_log.is_empty(), "hold should produce output");
    let has_pageup = s.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "pgup"));
    assert!(has_pageup, "mouseside1 hold should emit pgup");
}

#[test]
fn test_mouse_move_send_key() {
    // Test that send_key parses MouseMove(x,y) correctly
    let mut s = PipelineState::new(test_config());
    let mut ctx = Context::new("test");
    ctx.logical_key = "MouseMove(50, -30)".to_string();
    s.send_key(&mut ctx);
    let has_mm = s.emit_log.iter().any(|e| matches!(e, EmitEvent::MouseMove(50, -30, _)));
    assert!(has_mm, "MouseMove(50,-30) should produce MouseMove event");
}

#[test]
fn test_defer_interrupts_doubletap() {
    let mut cfg = test_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("space".into(), KeyEntry {
        tap: "{space}".into(),
        hold: "{fn1}".into(),
        ht: "200".into(),
        dt: "{capslock}".into(),
        dtt: "250".into(),
        ..Default::default()
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("z".into(), KeyEntry {
        tap: "{z}".into(),
        hold: "{shift}".into(),
        ht: "200".into(),
        ..Default::default()
    });
    let mut s = new_state(cfg);
    s.debug_enabled = true;

    s.key_down("space");
    assert_eq!(s.state.keys.key_states["space"].td.as_ref().unwrap().kind, TdKind::Waiting);

    s.key_down("z");
    assert!(s.state.defer.waiting_stack.iter().any(|(k, _)| k == "space"),
        "space should be on waiting_stack");

    s.key_up("space");
    assert_eq!(s.state.keys.key_states["space"].td.as_ref().unwrap().kind, TdKind::TapDone,
        "space should be TapDone after up");

    s.emit_log.clear();
    s.key_up("z");
    let space_tap = s.emit_log.iter().any(|e| {
        matches!(e, EmitEvent::Down(k, _) if k == "space")
    });
    assert!(space_tap, "z release should have interrupted space doubletap");
}

// ═══════════════════════════════════════════
// Combo 重构（Stage 2+3）：匹配 → P4 激活层/flush → P7 映射 → P8 发送
// ═══════════════════════════════════════════

/// 单键 combo（输出为单个 {X}）→ 转 Holding；长按自动重复应再发一次。
#[test]
fn test_single_key_combo_auto_repeat() {
    let mut cfg = test_config();
    cfg.combo_map.push(ComboRow {
        key1: "a".into(), key2: "s".into(),
        output: "{F1}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry { tap: "{a}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("s".into(), KeyEntry { tap: "{s}".into(), ..Default::default() });
    let mut s = new_state(cfg);

    s.key_down("a");
    s.key_down("s"); // 匹配 → P7 解析 {F1}, is_single_key=true → Holding
    let after_match = s.emit_log.iter().filter(|e| matches!(e, EmitEvent::Down(k, _) if k == "F1")).count();
    assert_eq!(after_match, 1, "combo 匹配应发出 1 次 {{F1}}");
    assert_eq!(s.state.keys.key_states["a"].combo.as_ref().unwrap().state, ComboKind::Holding,
               "单键 combo 应转 Holding（供 Phase2 重发）");

    // 模拟 OS 长按自动重复：再次 key_down 已按住的 'a'
    s.key_down("a");
    let after_repeat = s.emit_log.iter().filter(|e| matches!(e, EmitEvent::Down(k, _) if k == "F1")).count();
    assert_eq!(after_repeat, 2, "长按单键 combo 应自动重复 → 第 2 次 {{F1}}");
}

/// 多键 macro combo（输出 {a}{b}{c}）→ 转 Active；长按只发一次，不重复。
#[test]
fn test_macro_combo_fires_once() {
    let mut cfg = test_config();
    cfg.combo_map.push(ComboRow {
        key1: "q".into(), key2: "w".into(),
        output: "{a}{b}{c}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("q".into(), KeyEntry { tap: "{q}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("w".into(), KeyEntry { tap: "{w}".into(), ..Default::default() });
    let mut s = new_state(cfg);

    s.key_down("q");
    s.key_down("w"); // 匹配 → P7 解析 {a}{b}{c}, is_single_key=false → Active
    assert_eq!(s.state.keys.key_states["q"].combo.as_ref().unwrap().state, ComboKind::Active,
               "多键 combo 应转 Active（不重发）");
    // 多键 macro 输出作为单个 Text 事件发出（非逐键 Down）
    let macro_match = s.emit_log.iter().filter(|e| matches!(e, EmitEvent::Text(t, _) if t == "{a}{b}{c}")).count();
    assert!(macro_match >= 1, "macro 应发出 1 次 {{a}}{{b}}{{c}}（Text）");

    // 模拟长按自动重复：再次 key_down 'q'
    s.key_down("q");
    let macro_repeat = s.emit_log.iter().filter(|e| matches!(e, EmitEvent::Text(t, _) if t == "{a}{b}{c}")).count();
    assert_eq!(macro_repeat, macro_match, "macro combo 长按不应重复 → Text 计数不变");
}

/// 层 combo：先让 switch 键进入 waiting_stack（未越过 hold_term），再按 a+s 应正确触发 {X}，
/// 且层激活事件不晚于 combo 输出（验证 §6 顺序颠倒 bug 已修复，由 phase4_flush 无条件激活 waiting switch）。
#[test]
fn test_layer_combo_triggers_and_order() {
    let mut cfg = test_config();
    // space hold → {fn1} 作为层切换开关（进入 waiting_stack 的 switch/修饰键）
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("space".into(), KeyEntry {
        tap: "{space}".into(), hold: "{fn1}".into(), ht: "150".into(), ..Default::default()
    });
    // fn1 层内 combo: a+s → {X}
    cfg.combo_map.push(ComboRow {
        key1: "a".into(), key2: "s".into(),
        output: "{X}".into(), layer: "fn1".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry { tap: "{a}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("s".into(), KeyEntry { tap: "{s}".into(), ..Default::default() });
    let mut s = new_state(cfg);

    // 1) space 按下（进入 waiting_stack，尚未越过 hold_term → fn1 尚未由 hold_timer 激活）
    s.key_down("space");
    assert!(s.state.defer.waiting_stack.iter().any(|(k, _)| k == "space"), "space 应在 waiting_stack（Waiting）");
    assert_eq!(s.current_layer(), "base", "hold_term 未到，fn1 不应激活");

    // 2) space 仍为 Waiting 时按 a+s → 匹配层 combo；
    //    phase4_flush 在 combo 命中时 force-hold space 激活 fn1，phase7 才能查到 {X}
    s.key_down("a");
    s.key_down("s");
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "X")),
            "层 combo 应在 fn1 正确触发 {{X}}");
    assert_eq!(s.current_layer(), "fn1", "combo 命中应通过 phase4_flush 激活 fn1");
    assert!(!s.state.defer.waiting_stack.iter().any(|(k, _)| k == "space"),
            "phase4_flush force-hold 后 space 应移出 waiting_stack");

    // 3) 顺序：层（修饰键）激活不应晚于 combo 输出（§6 修复点）
    let layer_on_idx = s.emit_log.iter().position(|e| matches!(e, EmitEvent::LayerOn(n, _) if n == "fn1"));
    let x_idx = s.emit_log.iter().position(|e| matches!(e, EmitEvent::Down(k, _) if k == "X"));
    assert!(layer_on_idx.is_some() && x_idx.is_some(), "应同时存在 LayerOn(fn1) 与 Down X");
    assert!(layer_on_idx.unwrap() <= x_idx.unwrap(),
            "层激活不应晚于 combo 输出（修复 §6 输出顺序颠倒）");
}

/// fn+shift+组合（层 combo），三者快速按下（无 hold_term 等待）：验证 shift 修饰键被发出并**包裹**层 combo 输出，
/// 形成 Shift+Home 这样的修饰组合（Down(s) ≤ Down(Home) ≤ Up(Home) ≤ Up(s)）。
/// 这是 stack_layer_mod_combo_fast 场景的 Rust 侧等价断言，覆盖「带 shift 的层 combo」快速路径。
#[test]
fn test_fn_shift_combo_emits_shift_modified() {
    let mut cfg = test_config();
    // fn 层键 f：hold→{fn1}，hold_term 大，快速按下不越过 → 进 waiting_stack（Waiting）
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{fn1}".into(), ht: "600".into(), ..Default::default()
    });
    // shift 修饰键 s：定义在 fn1 层（hold→{shift}），层内修饰键供「带 shift 的层 combo」。
    // 注意：必须放在目标层 fn1 的 layer_defs —— 放 base_layer 时，fn1 激活后 current_layer=fn1，
    // is_switch_key 查 k["s"]["fn1"] 缺失且无 base 回退，s 不再被识别为修饰键，shift 不会激活。
    cfg.layers.layer_maps.insert("fn1".into(), {
            let mut m = HashMap::new();
            m.insert("s".into(), KeyEntry {
                tap: "{s}".into(), hold: "{shift}".into(), ht: "600".into(), ..Default::default()
            });
            m
        });
    // fn1 层内 combo: h+j → {Home}
    cfg.combo_map.push(ComboRow {
        key1: "h".into(), key2: "j".into(),
        output: "{Home}".into(), layer: "fn1".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("h".into(), KeyEntry { tap: "{h}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("j".into(), KeyEntry { tap: "{j}".into(), ..Default::default() });
    let mut s = new_state(cfg);

    // 全部快速按下：f↓ s↓ h↓ j↓（无 advance_ms）
    s.key_down("f");
    assert!(s.state.defer.waiting_stack.iter().any(|(k, _)| k == "f"), "f 应在 waiting_stack（Waiting）");
    s.key_down("s");
    // s 定义在 fn1 层（hold=shift）→ 作为层内修饰键进入 waiting_stack；combo 命中后由 phase4_flush
    // force-hold 成 shift 修饰键（发出 Down(shift)），而非被当普通键 tap 发 Down(s)。
    s.key_down("h");
    s.key_down("j"); // 匹配层 combo h+j→{Home}；phase4_flush 激活 f→fn1 且 force-hold s→shift

    // 1) key_down 阶段：层 combo 触发 + shift 修饰键发出，且 shift Down 不晚于 Home Down（修饰前置）
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "Home")),
            "fn+shift+combo 应正确触发层 combo {{Home}}");
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "shift")),
            "shift 修饰键应被发出（Down shift，而非 tap 的 Down s）");
    let s_down  = s.emit_log.iter().position(|e| matches!(e, EmitEvent::Down(k, _) if k == "shift"));
    let home_dn = s.emit_log.iter().position(|e| matches!(e, EmitEvent::Down(k, _) if k == "Home"));
    assert!(s_down.is_some() && home_dn.is_some(), "应同时存在 Down shift 与 Down Home");
    assert!(s_down.unwrap() < home_dn.unwrap(),
            "shift Down 应先于 Home Down（修饰前置，形成 Shift+Home）");

    // 1b) 快速释放：j↑ h↑ s↑ f↑（释放后校验完整包裹）
    s.key_up("j");
    s.key_up("h");
    s.key_up("s");
    s.key_up("f");

    // 3) 释放后：Home / shift 的 Up 存在，且整体包裹顺序 Down(shift) ≤ Down(Home) ≤ Up(Home) ≤ Up(shift)
    let home_up = s.emit_log.iter().position(|e| matches!(e, EmitEvent::Up(k, _) if k == "Home"));
    let s_up    = s.emit_log.iter().position(|e| matches!(e, EmitEvent::Up(k, _) if k == "shift"));
    assert!(home_up.is_some() && s_up.is_some(), "应同时存在 Up Home 与 Up shift");
    assert!(home_up.unwrap() < s_up.unwrap(),
            "Home Up 应先于 shift Up（修饰后置包裹）");
    // 完整包裹链校验
    assert!(s_down.unwrap() < home_dn.unwrap()
            && home_dn.unwrap() < home_up.unwrap()
            && home_up.unwrap() < s_up.unwrap(),
            "应形成完整包裹：Down(shift) ≤ Down(Home) ≤ Up(Home) ≤ Up(shift)");
    assert_eq!(s.current_layer(), "base", "全部释放后层应回到 base");
}

/// 回归测试：fn(f, hold={fn1}) 之后快速按 shift(s, hold={shift}@fn1) 时，
/// s 必须进入 waiting_stack（此前 bug：current_layer 仍是 base，s 在 base 无定义，
/// tap_dance_down 直接 return，s 进不了 waiting_stack，被当普通键 tap 发 Down(s)）。
/// 这是「层内修饰键在层键 Waiting 期间按下」进入等待栈的直接断言。
#[test]
fn test_layer_modifier_enters_waiting_stack() {
    let mut cfg = test_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{fn1}".into(), ht: "600".into(), ..Default::default()
    });
    cfg.layers.layer_maps.insert("fn1".into(), {
            let mut m = HashMap::new();
            m.insert("s".into(), KeyEntry {
                tap: "{s}".into(), hold: "{shift}".into(), ht: "600".into(), ..Default::default()
            });
            m
        });
    let mut s = new_state(cfg);

    s.key_down("f"); // 层键，Waiting
    assert!(s.state.defer.waiting_stack.iter().any(|(k, _)| k == "f"), "f 应在 waiting_stack（Waiting）");
    s.key_down("s"); // fn1 内 hold={shift}，紧跟 f 快速按下
    assert!(s.is_switch_key("s"), "s 应被识别为修饰键（is_switch_key）");
    assert!(s.state.defer.waiting_stack.iter().any(|(k, _)| k == "s"),
            "fn1 内 hold={{shift}} 修饰键在层键 f 之后快速按下时应进入 waiting_stack");
}

/// fn + 物理 shift 键（lshift）+ 层 combo，三者快速按下：验证物理 shift 修饰键被发出并**包裹**层 combo 输出，
/// 形成 Shift+Home（Down(lshift) ≤ … ≤ Down(Home) ≤ Up(Home) ≤ … ≤ Up(lshift)）。
/// 与 test_fn_shift_combo_emits_shift_modified 的区别：此处用真实物理 shift 键（lshift），而非 remap 的 s 键。
#[test]
fn test_fn_physical_shift_combo_emits_shift_modified() {
    let mut cfg = test_config();
    // fn 层键 f：hold→{fn1}，hold_term 大，快速按下不越过 → 进 waiting_stack（Waiting）
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry {
        tap: "{f}".into(), hold: "{fn1}".into(), ht: "600".into(), ..Default::default()
    });
    // 物理 shift 键 lshift：定义在 fn1 层（hold→{shift}），层内修饰键供「带 shift 的层 combo」
    cfg.layers.layer_maps.insert("fn1".into(), {
            let mut m = HashMap::new();
            m.insert("lshift".into(), KeyEntry {
                tap: "{lshift}".into(), hold: "{shift}".into(), ht: "600".into(), ..Default::default()
            });
            m
        });
    // fn1 层内 combo: h+j → {Home}
    cfg.combo_map.push(ComboRow {
        key1: "h".into(), key2: "j".into(),
        output: "{Home}".into(), layer: "fn1".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("h".into(), KeyEntry { tap: "{h}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("j".into(), KeyEntry { tap: "{j}".into(), ..Default::default() });
    let mut s = new_state(cfg);

    // 全部快速按下：f↓ lshift↓ h↓ j↓（无 advance_ms）
    s.key_down("f");
    assert!(s.state.defer.waiting_stack.iter().any(|(k, _)| k == "f"), "f 应在 waiting_stack（Waiting）");
    s.key_down("lshift");
    // 物理 shift 定义在 fn1 层（hold=shift）→ 作为层内修饰键进入 waiting_stack；combo 命中后由
    // phase4_flush force-hold 成 shift 修饰键（发出 Down(shift)），而非被当普通键 tap 发 Down(lshift)。
    assert!(s.state.defer.waiting_stack.iter().any(|(k, _)| k == "lshift"),
            "物理 shift（lshift）在层键 f 之后应进入 waiting_stack");
    s.key_down("h");
    s.key_down("j"); // 匹配层 combo h+j→{Home}；phase4_flush 激活 f→fn1 且 force-hold lshift→shift

    // 1) 层 combo 触发 + 物理 shift 修饰键发出（逻辑 shift）+ fn1 激活
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "Home")),
            "fn+物理shift+combo 应正确触发层 combo {{Home}}");
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "shift")),
            "物理 shift 修饰键应被发出（Down shift，而非 tap 的 Down lshift）");
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::LayerOn(n, _) if n == "fn1")),
            "fn1 层应被激活（层 combo 命中的前提）");

    // 2) key_down 阶段包裹：shift Down 先于 Home Down（修饰前置，形成 Shift+Home）
    let ls_down  = s.emit_log.iter().position(|e| matches!(e, EmitEvent::Down(k, _) if k == "shift"));
    let home_dn = s.emit_log.iter().position(|e| matches!(e, EmitEvent::Down(k, _) if k == "Home"));
    assert!(ls_down.is_some() && home_dn.is_some(), "应同时存在 Down(shift) 与 Down(Home)");
    assert!(ls_down.unwrap() < home_dn.unwrap(),
            "shift Down 应先于 Home Down（修饰前置，形成 Shift+Home）");

    // 3) 快速释放：j↑ h↑ lshift↑ f↑
    s.key_up("j");
    s.key_up("h");
    s.key_up("lshift");
    s.key_up("f");

    // 4) 释放后：整体包裹顺序 Down(shift) ≤ Down(Home) ≤ Up(Home) ≤ Up(shift)
    let home_up = s.emit_log.iter().position(|e| matches!(e, EmitEvent::Up(k, _)   if k == "Home"));
    let ls_up   = s.emit_log.iter().position(|e| matches!(e, EmitEvent::Up(k, _)   if k == "shift"));
    assert!(home_up.is_some() && ls_up.is_some(), "应同时存在 Up(Home) 与 Up(shift)");
    assert!(home_up.unwrap() < ls_up.unwrap(),
            "Home Up 应先于 shift Up（修饰后置包裹）");
    assert!(ls_down.unwrap() < home_dn.unwrap()
            && home_dn.unwrap() < home_up.unwrap()
            && home_up.unwrap() < ls_up.unwrap(),
            "应形成完整包裹：Down(shift) ≤ Down(Home) ≤ Up(Home) ≤ Up(shift)");

    // 5) 层激活/反激活与最终层
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::LayerOff(n, _) if n == "fn1")),
            "应反激活 fn1 层");
    assert_eq!(s.current_layer(), "base", "全部释放后层应回到 base");
}

/// 回归：hold-combo（a+b → {c}）的 Up 必须在 final 阶段发送，且发送后彻底清理，
/// 不留 emitted/target_layer 残留。锁定「phase_up2 只写状态、发送放 final、发送后清残留」架构。
#[test]
fn test_hold_combo_emits_in_final_and_cleans_residue() {
    let mut cfg = test_config();
    cfg.combo_map.push(ComboRow {
        key1: "a".into(), key2: "b".into(),
        output: "{c}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry { tap: "{a}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("b".into(), KeyEntry { tap: "{b}".into(), ..Default::default() });
    let mut s = new_state(cfg);

    s.key_down("a");
    s.key_down("b");   // combo 命中 → 发 Down(c)，emitted 记录组合输出在本键
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "c")),
            "combo 应发出 Down(c)");

    // 先后抬起两键（无论谁先，最后抬起者负责发 Up）
    s.key_up("a");
    s.key_up("b");

    // Up(c) 恰发一次（由 final 的 release_key 发送，非 phase_up2）
    let up_c = s.emit_log.iter().filter(|e| matches!(e, EmitEvent::Up(k, _) if k == "c")).count();
    assert_eq!(up_c, 1, "hold-combo 的 Up(c) 应只发一次");

    // 无残留：两键的 emitted 与 target_layer 都应已清空
    assert!(s.emitted("a").is_none(), "a 的 emitted 应已清理");
    assert!(s.emitted("b").is_none(), "b 的 emitted 应已清理");
    assert!(s.target_layer("a").is_none(), "a 的 target_layer 应已清理");
    assert!(s.target_layer("b").is_none(), "b 的 target_layer 应已清理");

    // combo 状态本身也已回收（两键都 Released 后由 phase_up5_final 收尾）
    assert!(!s.has_combo("a"), "a 的 combo 状态应已回收");
    assert!(!s.has_combo("b"), "b 的 combo 状态应已回收");
}

// ───────────────────────── Leader Key 测试（Step4/5）─────────────────────────

fn leader_base_cfg() -> Config {
    let mut cfg = test_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("f".into(), KeyEntry { tap: "{leader}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry { tap: "{a}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("b".into(), KeyEntry { tap: "{b}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("c".into(), KeyEntry { tap: "{c}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("j".into(), KeyEntry { tap: "{j}".into(), ..Default::default() });
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("k".into(), KeyEntry { tap: "{k}".into(), ..Default::default() });
    cfg
}

fn downs(s: &PipelineState) -> Vec<String> {
    s.emit_log.iter().filter_map(|e| match e {
        EmitEvent::Down(k, _) => Some(k.clone()),
        _ => None,
    }).collect()
}

/// 收集 emit_log 中的 EmitEvent::Text（SendInput unicode 文本输出）。
/// Leader 单字符键输出（{x}/{3}/…）现经 emit_text 发出，故断言其出现需查此处。
fn texts(s: &PipelineState) -> Vec<String> {
    s.emit_log.iter().filter_map(|e| match e {
        EmitEvent::Text(t, _) => Some(t.clone()),
        _ => None,
    }).collect()
}

/// 收集 emit_log 中的 EmitEvent::TapSI（SendInput 扫描码 tap，含 E0）。
/// Leader 控制键名输出（{esc}/{enter}/{end}/…）现经 emit_tap_si 发出，断言其出现查此处。
fn tapsi(s: &PipelineState) -> Vec<String> {
    s.emit_log.iter().filter_map(|e| match e {
        EmitEvent::TapSI(k, _) => Some(k.clone()),
        _ => None,
    }).collect()
}

#[test]
fn test_leader_blocking_match() {
    // {leader} + a b → 完成键 b 抬起后触发 {esc}，期间 a/b/f 输出被吞
    let mut cfg = leader_base_cfg();
    cfg.leader.sequences = vec![LeaderSequence {
        keys: vec!["{a}".into(), "{b}".into()],
        output: "{esc}".into(),
        ..Default::default()
    }];
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");   // 触发 {leader}，进入拦截态（f 被吞）
    s.key_down("a"); s.key_up("a");   // 记录 a（被吞）
    s.key_down("b"); s.key_up("b");   // 记录 b，匹配 [a,b] → 触发 {esc}

    let d = downs(&s);
    let ts = tapsi(&s);
    assert!(ts.contains(&"esc".to_string()), "blocking 下 action esc 应经 TapSI(SendInput 扫描码) 发出");
    assert_eq!(d, Vec::<String>::new(), "blocking 下被吞键不应经 Down 事件出现");
    assert!(!d.contains(&"a".to_string()) && !d.contains(&"b".to_string()) && !d.contains(&"f".to_string()),
        "被拦截键不应出现在输出中");
    // 拦截态收尾：blk 标记应已清（b 抬起时清），无残留 leader_on
    assert!(!s.state.leader.leader_on, "触发后 leader_on 应已关闭（本次运行结束）");
}

#[test]
fn test_leader_blocking_match_jk() {
    // 固定触发键 {leader}（blocking）：按 f 触发后 j/k 被吞，k 抬起后触发 {esc}
    let mut cfg = leader_base_cfg();
    cfg.leader.sequences = vec![LeaderSequence {
        keys: vec!["{j}".into(), "{k}".into()],
        output: "{esc}".into(),
        ..Default::default()
    }];
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");   // 触发 {leader}，进入拦截态（f 被吞）
    s.key_down("j"); s.key_up("j");   // 记录 j（被吞）
    s.key_down("k"); s.key_up("k");   // 记录 k，匹配 [j,k] → 触发 {esc}

    let d = downs(&s);
    let ts = tapsi(&s);
    assert!(!d.contains(&"j".to_string()), "blocking 下 j 应被吞");
    assert!(!d.contains(&"k".to_string()), "blocking 下 k 应被吞");
    assert!(ts.contains(&"esc".to_string()), "k 抬起后 action esc 应经 TapSI 发出");
    assert!(!s.state.leader.leader_on, "触发后 leader_on 应已关闭");
}

#[test]
fn test_leader_deadend_no_trigger() {
    // blocking（固定 {leader} 触发）：a→c 不是任何序列（[a,b] 才是）→ dead-end 清栈，不误触发
    let mut cfg = leader_base_cfg();
    cfg.leader.sequences = vec![LeaderSequence {
        keys: vec!["{a}".into(), "{b}".into()],
        output: "{esc}".into(),
        ..Default::default()
    }];
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");   // 触发 {leader}
    s.key_down("a"); s.key_up("a");   // [a] 是 [a,b] 前缀 → 等待
    s.key_down("c"); s.key_up("c");   // [a,c] dead-end → 清栈，c 被吞，不触发

    let d = downs(&s);
    assert!(!d.contains(&"a".to_string()), "blocking 下 a 应被吞");
    assert!(!d.contains(&"c".to_string()), "blocking 下 c 应被吞");
    assert!(!d.contains(&"esc".to_string()), "dead-end 不应触发 action");
    assert!(s.state.leader.leader_stack.is_empty(), "dead-end 后栈应清空");
    assert!(!s.state.leader.leader_on, "dead-end 应退出 blocking 拦截态（用户不被卡住）");
}

#[test]
fn test_leader_longest_overlap_fires_short_then_long() {
    // 重叠序列 [a,b]→X 与 [a,b,c]→Y：按 a b 立即触发短序列 X（exact 命中即 fire），
    // 但栈保留（因 [a,b] 仍是 [a,b,c] 前缀）→ 继续等 c；按 c 后触发 Y。
    // blocking 下续接键 c 仍被吞（leader_on 在栈保留期间保持）。
    let mut cfg = leader_base_cfg();
    cfg.leader.sequences = vec![
        LeaderSequence { keys: vec!["{a}".into(), "{b}".into()], output: "{x}".into(), ..Default::default() },
        LeaderSequence { keys: vec!["{a}".into(), "{b}".into(), "{c}".into()], output: "{y}".into(), ..Default::default() },
    ];
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");       // 触发 {leader}
    s.key_down("a"); s.key_up("a");
    s.key_down("b"); s.key_up("b");       // [a,b] exact 命中 → 立即触发 X；栈保留等更长
    let si_after_b = tapsi(&s);
    assert!(si_after_b.contains(&"x".to_string()), "a b 后应立刻触发短序列 X（exact 命中即 fire，单字符经 TapSI 输出）");
    assert!(!si_after_b.contains(&"y".to_string()), "a b 后尚未完成，不应触发 Y");
    assert_eq!(s.state.leader.leader_stack.len(), 2, "a b 后栈应保留 [a,b] 以续接更长序列");
    assert!(s.state.leader.leader_on, "栈保留期间 blocking 拦截态应保持（继续等）");

    s.key_down("c"); s.key_up("c");       // 延伸为 [a,b,c] → 触发 Y（最长匹配），清栈
    let si = tapsi(&s);
    assert!(si.contains(&"x".to_string()), "应已触发 X");
    assert!(si.contains(&"y".to_string()), "完成最长序列后应触发 Y");
    assert!(!s.state.leader.leader_on, "完整序列结束后 blocking 拦截态应关闭");
}

#[test]
fn test_leader_loop_capture_chains() {
    // 循环捕获（loop_capture=true, non-blocking）：按 1 2 触发 {3}，输出回灌驱动串联
    // → 自动续出 4 → 5 → bingo!（无需再物理按键）。
    let mut cfg = leader_base_cfg();
    cfg.leader.loop_capture = true;
    cfg.leader.sequences = vec![
        LeaderSequence { keys: vec!["{1}".into(), "{2}".into()], output: "{3}".into(), ..Default::default() },
        LeaderSequence { keys: vec!["{1}".into(), "{2}".into(), "{3}".into()], output: "{4}".into(), ..Default::default() },
        LeaderSequence { keys: vec!["{1}".into(), "{2}".into(), "{3}".into(), "{4}".into()], output: "{5}".into(), ..Default::default() },
        LeaderSequence { keys: vec!["{1}".into(), "{2}".into(), "{3}".into(), "{4}".into(), "{5}".into()], output: "bingo!".into(), ..Default::default() },
    ];
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");   // 触发 {leader}（blocking）
    s.key_down("1"); s.key_up("1");
    s.key_down("2"); s.key_up("2");   // 触发 {3}，级联自动续出 4 5 bingo!

    let t = texts(&s);
    let si = tapsi(&s);
    assert!(si.iter().any(|x| x == "3"), "循环捕获下 1 2 应触发 3（单字符经 TapSI 输出）");
    assert!(si.iter().any(|x| x == "4"), "输出回灌应自动续出 4");
    assert!(si.iter().any(|x| x == "5"), "应自动续出 5");
    assert!(t.iter().any(|x| x == "bingo!"), "应自动级联到 bingo!（文本输出）");
    // 级联应终止，栈被最终序列清掉
    assert!(s.state.leader.leader_stack.is_empty(), "级联结束后栈应为空");
}

#[test]
fn test_leader_no_loop_capture_no_chain() {
    // 默认（loop_capture=false）：按 1 2 只触发 {3}，不自动续接（维持原版行为）。
    let mut cfg = leader_base_cfg();
    cfg.leader.loop_capture = false;
    cfg.leader.sequences = vec![
        LeaderSequence { keys: vec!["{1}".into(), "{2}".into()], output: "{3}".into(), ..Default::default() },
        LeaderSequence { keys: vec!["{1}".into(), "{2}".into(), "{3}".into()], output: "{4}".into(), ..Default::default() },
    ];
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");   // 触发 {leader}（blocking）
    s.key_down("1"); s.key_up("1");
    s.key_down("2"); s.key_up("2");

    let t = texts(&s);
    let si = tapsi(&s);
    assert!(si.iter().any(|x| x == "3"), "1 2 应触发 3（单字符经 TapSI 输出）");
    assert!(!t.iter().any(|x| x == "4"), "默认不开启循环捕获：输出不应回灌，故不续出 4");
}

#[test]
fn test_leader_blocking_timeout_clears_stack() {
    // blocking：触发 {leader} + a 后超时 → 栈清空、leader_on 关闭；后续 b 不再被吞、不触发
    let mut cfg = leader_base_cfg();
    cfg.leader.timeout_ms = 100;
    cfg.leader.sequences = vec![LeaderSequence {
        keys: vec!["{a}".into(), "{b}".into()],
        output: "{esc}".into(),
        ..Default::default()
    }];
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");   // 触发 {leader}，leader_on=true，启动滑动窗口
    s.key_down("a"); s.key_up("a");   // 记录 a，刷新窗口
    assert_eq!(s.state.leader.leader_stack.len(), 1, "按下 a 后栈应为 [a]");
    assert!(s.state.leader.leader_on, "触发后 leader_on 应为 true");

    s.advance_ms(150);                // 超过 timeout_ms → 滑动超时到期
    assert!(s.state.leader.leader_stack.is_empty(), "超时后栈应清空");
    assert!(!s.state.leader.leader_on, "超时后 blocking leader_on 应关闭");
    let d_timeout = downs(&s);
    assert!(!d_timeout.contains(&"esc".to_string()), "超时未匹配，不应触发 action");

    // 超时后续按 b：leader_on 已关，b 不被吞、不触发（序列作废）
    s.key_down("b"); s.key_up("b");
    let d2 = downs(&s);
    assert!(d2.contains(&"b".to_string()), "超时后 b 应正常发出（不被吞）");
    assert!(!d2.contains(&"esc".to_string()), "超时后 b 不应触发 action（序列已作废）");
}

#[test]
fn test_leader_blocking_timeout_then_b_passes() {
    // blocking（固定 {leader} 触发）：触发 + a 后超时 → 栈清空、leader_on 关闭；后续 b 照发、不触发
    let mut cfg = leader_base_cfg();
    cfg.leader.timeout_ms = 100;
    cfg.leader.sequences = vec![LeaderSequence {
        keys: vec!["{a}".into(), "{b}".into()],
        output: "{esc}".into(),
        ..Default::default()
    }];
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");   // 触发 {leader}
    s.key_down("a"); s.key_up("a");   // [a] 是 [a,b] 前缀 → 等待，启动窗口（a 被吞）
    s.advance_ms(150);                // 超时 → 栈清空、leader_on 关闭
    assert!(s.state.leader.leader_stack.is_empty(), "blocking 超时后栈应清空");

    s.key_down("b"); s.key_up("b");   // 超时后 leader_on 已关，b 走 pass-through 照发
    let d = downs(&s);
    assert!(!d.contains(&"a".to_string()), "blocking 下 a 应被吞");
    assert!(d.contains(&"b".to_string()), "超时后 b 应照发");
    assert!(!d.contains(&"esc".to_string()), "超时后 b 不应触发 action");
}

#[test]
fn test_leader_layer_key_exempt() {
    // §10：层键（{fn1}）在 leader 记录/拦截中豁免——不进栈、照常激活层，序列永远无法匹配
    let mut cfg = leader_base_cfg();
    cfg.leader.timeout_ms = 100;
    cfg.leader.sequences = vec![LeaderSequence {
        keys: vec!["{a}".into(), "{fn1}".into()],
        output: "{esc}".into(),
        ..Default::default()
    }];
    // 让 g 输出层键 {fn1}
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("g".into(), KeyEntry { tap: "{fn1}".into(), ..Default::default() });
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");   // 触发 {leader}
    s.key_down("a"); s.key_up("a");   // 记录 a（被吞）
    s.key_down("g"); s.key_up("g");   // g 输出 {fn1} → 层键豁免，不进栈、照常激活层

    assert_eq!(s.state.leader.leader_stack.len(), 1, "层键不应进栈");
    assert_eq!(s.state.leader.leader_stack[0].1, "{a}", "栈中应为 a");
    let d = downs(&s);
    assert!(!d.contains(&"esc".to_string()), "含层键的序列因豁免永远无法匹配，不应触发");
}

#[test]
fn test_leader_control_key_routes_to_tapsi() {
    // 补全：leader 合成键输出控制键名（{enter}/{end}/{f1}…）必须走 SendInput 扫描码（TapSI），
    // 与 unicode 文本同处 RIT 输入流，否则会回退 Down/Up 事件通道、与 leader 文本交错
    // （现象 12B3i4n5go!!! 的残留来源）。
    let mut s = new_state(test_config());

    // {enter} → raw="enter"（多字符）→ TapSI("enter")
    let mut ctx = Context { key: "__leader_out".into(), logical_key: "{enter}".into(), ..Default::default() };
    s.send_key(&mut ctx);
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::TapSI(k, _) if k == "enter")),
            "leader 控制键 enter 应走 TapSI（SendInput 扫描码）");

    // {end} → TapSI("end")（E0 扩展键）
    let mut ctx = Context { key: "__leader_out".into(), logical_key: "{end}".into(), ..Default::default() };
    s.send_key(&mut ctx);
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::TapSI(k, _) if k == "end")),
            "leader 控制键 end 应走 TapSI（SendInput 扫描码，含 E0）");

    // 单字符走 TapSI（扫描码可响应修饰键如 Shift），不走 Down/Up 通道
    let mut ctx = Context { key: "__leader_out".into(), logical_key: "{b}".into(), ..Default::default() };
    s.send_key(&mut ctx);
    assert!(s.emit_log.iter().any(|e| matches!(e, EmitEvent::TapSI(k, _) if k == "b")),
            "leader 单字符键 b 应走 TapSI（SendInput 扫描码，尊重 Shift 状态）");

    // 关键：整段 leader 输出不得出现任何 Down/Up/Tap 事件（应全部走 SendInput）
    let intercept_count = s.emit_log.iter().filter(|e| matches!(e, EmitEvent::Down(_, _) | EmitEvent::Up(_, _) | EmitEvent::Tap(_, _))).count();
    assert_eq!(intercept_count, 0, "leader 合成键输出不得经 Down/Up 事件通道（否则会与文本交错）");
}

#[test]
fn test_select_fence_between_flt_burst_and_si_suffix() {
    // {Select N} 的 FLT burst 与后缀 SI 文本之间必须恰好插一条 Fence 指令。
    // FLT→SI 方向没有队列背压，不隔开则后缀字符会插进还在排空的 FLT 流
    // （实测 hira@126.com{select 12}123456 → 后缀变 654321）。
    // Fence 曾两次做错位置（在 commit 里直接 sleep / 在 main 全局计数），本测试锁死
    // 「作为 emit_log 指令排在 burst 之后、suffix 之前」这一语义与时长公式。
    let mut s = new_state(test_config());
    let mut ctx = Context {
        key: "__leader_out".into(),
        logical_key: "hira@126.com{Select 12}123456".into(),
        ..Default::default()
    };
    s.send_key(&mut ctx);

    let fence_idx = s.emit_log.iter()
        .position(|e| matches!(e, EmitEvent::Fence(_)))
        .expect("leader 的 FLT burst + SI 后缀必须产生一条 Fence");
    assert_eq!(fence_idx, s.emit_log.len() - 2,
        "Fence 必须排在 FLT burst 之后、后缀 SI 之前（末尾两个事件是 Fence + suffix SI）");

    // 栅栏前紧邻 FLT burst 的收尾（shift UP，走 Down/Up 通道）
    assert!(matches!(s.emit_log[fence_idx - 1], EmitEvent::Up(ref k, _) if k == "shift"),
        "Fence 前一个事件应是 FLT burst 的 shift UP");

    // 时长公式：burst = 4N+2 事件（N=12 → 50），50 × 1.2 + 2 = 62
    assert!(matches!(s.emit_log[fence_idx], EmitEvent::Fence(62)),
        "Fence 时长应为积压 FLT 事件数 × 1.2 + 2ms");

    // 栅栏之后只允许 SI 变体（不得再混入 Down/Up 通道）
    assert!(s.emit_log[fence_idx + 1..].iter().all(|e| matches!(e,
        EmitEvent::Text(_, _) | EmitEvent::TapSI(_, _) | EmitEvent::DownSI(_, _) | EmitEvent::UpSI(_, _))),
        "Fence 之后应为纯 SI 输出");

    // 同通道后缀（非 leader + 简单 ASCII → 也走 FLT）不应产生栅栏
    let mut s2 = new_state(test_config());
    let mut ctx2 = Context { key: "x".into(), logical_key: "a{Select 3}bc".into(), ..Default::default() };
    s2.send_key(&mut ctx2);
    assert!(!s2.emit_log.iter().any(|e| matches!(e, EmitEvent::Fence(_))),
        "FLT→FLT 同通道切换无需栅栏（不应给所有输出加 fence）");
}

#[test]
fn test_leader_per_seq_timeout_max() {
    // 公共超时 1000；两条同前缀序列分别 100 / 500 → 滑动窗口应取候选最大 500。
    // 验证「每条不同超时」由引擎真正生效，且重叠前缀取最大。
    let mut cfg = leader_base_cfg();
    cfg.leader.timeout_ms = 1000;
    cfg.leader.sequences = vec![
        LeaderSequence { keys: vec!["{a}".into(), "{b}".into()], output: "{x}".into(), timeout_ms: 100, ..Default::default() },
        LeaderSequence { keys: vec!["{a}".into(), "{c}".into()], output: "{y}".into(), timeout_ms: 500, ..Default::default() },
    ];
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");   // 触发 {leader}
    s.key_down("a"); s.key_up("a");   // 栈=[a]；两条均为候选，max(100,500)=500 → 窗口 500ms
    s.advance_ms(300);                // 未到 500ms
    assert_eq!(s.state.leader.leader_stack.len(), 1, "300ms 内不应触发滑动超时");
    s.advance_ms(300);                // 累计 600ms > 500 → 滑动超时到期，清栈
    assert_eq!(s.state.leader.leader_stack.len(), 0, "超过 500ms 应触发滑动超时清栈（取候选最大超时）");
}

#[test]
fn test_leader_per_seq_timeout_fallback() {
    // 序列专属超时=0（未设）→ 回退公共超时。
    let mut cfg = leader_base_cfg();
    cfg.leader.timeout_ms = 300;
    cfg.leader.sequences = vec![
        LeaderSequence { keys: vec!["{a}".into(), "{b}".into()], output: "{x}".into(), timeout_ms: 0, ..Default::default() },
    ];
    let mut s = new_state(cfg);

    s.key_down("f"); s.key_up("f");   // 触发 {leader}
    s.key_down("a"); s.key_up("a");   // 专属超时=0 → 回退公共 300ms
    s.advance_ms(200);
    assert_eq!(s.state.leader.leader_stack.len(), 1, "200ms 内不应超时");
    s.advance_ms(200);                // 累计 400ms > 300 → 超时
    assert_eq!(s.state.leader.leader_stack.len(), 0, "应回退公共超时清栈");
}


#[test]
fn test_combo_identity_includes_layer() {
    // 回归：base a+s 与 fn2 a+s 是不同身份，merge 不互相覆盖
    let g = vec![
        ComboRow { key1: "a".into(), key2: "s".into(), output: "{left}".into(), layer: "base".into() },
        ComboRow { key1: "a".into(), key2: "s".into(), output: "{home}".into(), layer: "fn2".into() },
    ];
    // merge with itself (same layers) → 两条都保留
    let merged = anykey_engine::runtime_builder::merge_combos(&g, Some(&g));
    assert_eq!(merged.len(), 2, "base 与 fn2 的同键 combo 应共存（identity 含 layer）");
    let mut has_left = false;
    let mut has_home = false;
    for r in &merged {
        if r.output == "{left}" { has_left = true; }
        if r.output == "{home}" { has_home = true; }
    }
    assert!(has_left && has_home, "left={} home={}", has_left, has_home);
}
