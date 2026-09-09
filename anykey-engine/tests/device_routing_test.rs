/// AnyKey engine — Device routing tests (keyboard→mouse cross-type mapping)

use anykey_engine::state::*;
use anykey_engine::config::*;
use std::collections::HashMap;

fn base_config() -> Config {
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

// ── 键盘键 tap → 鼠标键，验证 emit_log 中 device_id = 触发键盘设备 ──

#[test]
fn test_keyboard_tap_maps_to_mouse_button() {
    let mut cfg = base_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{mouseleft}".into(),
        ..Default::default()
    });
    let mut s = new_state(cfg);

    // 模拟键盘设备 dev=7 按下 "a"
    s.current_device = 7;
    s.key_down("a");
    s.key_up("a");

    // 验证 emit_log 包含 MouseDown("mouseleft", 7)  和 MouseUp("mouseleft", 7)
    let mouse_downs: Vec<_> = s.emit_log.iter()
        .filter(|e| matches!(e, EmitEvent::MouseDown(_, _)))
        .collect();
    assert!(!mouse_downs.is_empty(), "should emit MouseDown for keyboard→mouse mapping");
    match mouse_downs[0] {
        EmitEvent::MouseDown(name, dev) => {
            assert_eq!(name, "mouseleft");
            assert_eq!(*dev, 7, "device_id should be keyboard device 7 (trigger device)");
        }
        other => panic!("Expected MouseDown, got {:?}", other),
    }

    let mouse_ups: Vec<_> = s.emit_log.iter()
        .filter(|e| matches!(e, EmitEvent::MouseUp(_, _)))
        .collect();
    assert!(!mouse_ups.is_empty(), "should emit MouseUp for keyboard→mouse mapping");
    match mouse_ups[0] {
        EmitEvent::MouseUp(name, dev) => {
            assert_eq!(name, "mouseleft");
            assert_eq!(*dev, 7, "device_id should be keyboard device 7 (trigger device)");
        }
        other => panic!("Expected MouseUp, got {:?}", other),
    }
}

// ── 中断场景：键 A（键盘 dev=7）被键 B（键盘 dev=9）打断 → tap 结算，
//    emit 的 device_id 仍应是 A 的设备 7 ──

#[test]
fn test_interrupt_preserves_trigger_device_id() {
    let mut cfg = base_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{mouseleft}".into(),
        ..Default::default()
    });
    let mut s = new_state(cfg);

    // 键 A 来自键盘 dev=7
    s.current_device = 7;
    s.key_down("a");

    // 键 B 来自键盘 dev=9（不同设备！）打断 A
    s.current_device = 9;
    s.key_down("b");

    // A 被中断，td Waiting → Finished，结算 tap → emit MouseDown("mouseleft", 7)
    let mouse_downs: Vec<_> = s.emit_log.iter()
        .filter(|e| matches!(e, EmitEvent::MouseDown(_, _)))
        .collect();
    assert!(!mouse_downs.is_empty(), "interrupted key should still emit MouseDown");
    match mouse_downs[0] {
        EmitEvent::MouseDown(name, dev) => {
            assert_eq!(name, "mouseleft");
            assert_eq!(*dev, 7, "device_id should be 7 (A's device), not 9 (B's device)");
        }
        other => panic!("Expected MouseDown, got {:?}", other),
    }
}

// ── 键盘键 hold → active，输出保持键盘设备 ID ──

#[test]
fn test_keyboard_hold_maps_to_mouse_button() {
    let mut cfg = base_config();
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(),
        hold: "{mouseleft}".into(),
        ..Default::default()
    });
    let mut s = new_state(cfg);

    // 键盘设备 dev=7 按下 "a"（hold_term=150ms）
    s.current_device = 7;
    s.key_down("a");

    // 推进到 hold_term 之后，触发 hold_timer（advance_ms 内部自动处理定时器）
    s.advance_ms(151);

    let mouse_downs: Vec<_> = s.emit_log.iter()
        .filter(|e| matches!(e, EmitEvent::MouseDown(_, _)))
        .collect();
    assert!(!mouse_downs.is_empty(), "hold timer should emit MouseDown");
    match mouse_downs[0] {
        EmitEvent::MouseDown(name, dev) => {
            assert_eq!(name, "mouseleft");
            assert_eq!(*dev, 7, "device_id should be 7 (keyboard), not lost in timer path");
        }
        other => panic!("Expected MouseDown, got {:?}", other),
    }
}
