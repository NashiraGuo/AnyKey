/// AnyKey engine — ForceHold TD state leak regression test
/// After a layer key is force-held then released, its td_state should be None,
/// not leaked Holding.  Regression for the bug in tap_dance_up: the !has_td
/// guard prevented Holding→Finished state transition for layer keys that
/// have no td config in the activated layer.

use anykey_engine::state::*;
use anykey_engine::config::*;
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

/// 层键被 force-hold 释放后 td_state 应为 None
///
/// 场景：
///   baseLayer: space(tap={space}, hold={fn1}, ht=200) → switch key
///   baseLayer: a(tap={a}, hold={a}, ht=200)          → has_td=true
///   fn1: a(tap={b}, hold={b}, ht=200)                → fn1 内有 a 配置使 target_anchor_layer 解析到后 a 能进 TD
///
/// 模拟流程：
///   1. DN space → Waiting（入 waiting_stack）
///   2. DN a     → Waiting（has_td=true）
///   3. UP a     → tap_dance_up: Waiting→Tap, force_hold_up=a
///                force_hold_execute(a): a不在栈→全激活→space force-hold→fn1激活
///                space: td_state=Holding, target_layer=fn1
///   4. UP space → release_key: 反激活 fn1
///                phase_up8_clean: 应清理 td_state（bug: Holding 泄露）
///   5. DN space → 正常进入 Waiting（泄露时被 Holding 分支吞噬）
#[test]
fn test_td_state_clean_after_forcehold_release() {
    let mut cfg = test_config();
    cfg.layers.hold_term = 200;
    // space: switch key (hold={fn1})
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("space".into(), KeyEntry {
        tap: "{space}".into(),
        hold: "{fn1}".into(),
        ht: "200".into(),
        ..Default::default()
    });
    // a: has_td=true (hold非空), 触发 force_hold
    cfg.layers.layer_maps.entry("base".into()).or_default().insert("a".into(), KeyEntry {
        tap: "{a}".into(),
        hold: "{a}".into(),
        ht: "200".into(),
        ..Default::default()
    });
    // fn1: no config for space (使 space UP 时 has_td=false), 
    // but has config for a (a 的 target_anchor_layer 由 space 解析到 fn1,
    // 须在 fn1 内有 a 配置 a 才能进 TD Waiting 状态)
    cfg.layers.layer_maps.insert("fn1".into(), {
        let mut m = std::collections::HashMap::new();
        m.insert("a".into(), KeyEntry {
            tap: "{b}".into(),
            hold: "{b}".into(),
            ht: "200".into(),
            ..Default::default()
        });
        m
    });

    let mut s = PipelineState::new(cfg);

    // ---- step 1: DN space ----
    s.key_down("space");
    assert!(s.td("space").is_some(), "space should be in TD state after DN");
    assert_eq!(s.td("space").unwrap().kind, TdKind::Waiting, "space should be Waiting");

    // ---- step 2: DN a ----
    s.key_down("a");
    assert!(s.td("a").is_some(), "a should be in TD state after DN");
    assert_eq!(s.td("a").unwrap().kind, TdKind::Waiting, "a should be Waiting");

    // ---- step 3: UP a → force-hold space, fn1 activated ----
    s.key_up("a");
    // a's td_state should be cleaned up
    assert!(s.td("a").is_none(), "a td_state should be None after UP");
    // space should now be Holding (force-held by a's UP)
    assert!(s.td("space").is_some(), "space should still have td_state after force-hold");
    assert_eq!(s.td("space").unwrap().kind, TdKind::Holding, "space should be Holding");
    // fn1 should be active
    assert_eq!(s.current_layer(), "fn1", "fn1 should be active after force-hold");

    // ---- step 4: UP space → release layer, should clean td_state ----
    s.key_up("space");
    // BUG: without fix, space's td_state leaked as Holding
    assert!(s.td("space").is_none(),
        "space td_state should be None after UP release, but was {:?}. \
         This is the Holding state leak bug: tap_dance_up guard skipped \
         Holding->Finished transition because !has_td in activated layer",
        s.td("space").map(|td| &td.kind));
    // fn1 should be deactivated
    assert_eq!(s.current_layer(), "base", "layer should return to base");

    // ---- step 5: DN space again → should work normally ----
    s.key_down("space");
    assert!(s.td("space").is_some(), "space should enter TD state again on second DN");
    // If Holding leaked, this would be some(Holding) and tap_dance_down would
    // return early with td_action=Holding → no Waiting, no hold timer.
    assert_eq!(s.td("space").unwrap().kind, TdKind::Waiting,
        "space should be Waiting on second DN (not leaked Holding)");
}
