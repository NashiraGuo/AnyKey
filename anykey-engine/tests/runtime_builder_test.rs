//! runtime_builder 单元测试（从 src/runtime_builder.rs 的 #[cfg(test)] 模块分离）。
//! 作为集成测试放在 tests/ 目录，仅可访问 pub API。

use anykey_engine::config::{AppOverride, ComboRow, Config, DeviceInfo, DeviceOverride, KeyEntry, LeaderSequence};
use anykey_engine::state::{ComboIndex, DeviceMapping, LeaderDef};
use anykey_engine::runtime_builder::*;
use std::collections::HashMap;

fn base_cfg() -> Config {
    Config {
        combo_time: 200,
        combo_map: vec![ComboRow { key1: "a".into(), key2: "b".into(), output: "X".into(), layer: "base".into() }],
        layers: Default::default(),
        leader: Default::default(),
        subscribed_devices: vec![],
        devices: HashMap::new(),
        app_aware: Default::default(),
        per_device: false,
    }
}

#[test]
fn merge_combos_override_by_trigger() {
    let g = vec![ComboRow { key1: "a".into(), key2: "b".into(), output: "X".into(), layer: "base".into() }];
    let ov = vec![ComboRow { key1: "b".into(), key2: "a".into(), output: "Y".into(), layer: "base".into() }];
    let merged = merge_combos(&g, Some(&ov));
    assert_eq!(merged.len(), 1, "同身份应合并为 1 条");
    assert_eq!(merged[0].output, "Y", "设备覆盖应胜出");
}

#[test]
fn merge_combos_append_distinct() {
    let g = vec![ComboRow { key1: "a".into(), key2: "b".into(), output: "X".into(), layer: "base".into() }];
    let ov = vec![ComboRow { key1: "c".into(), key2: "d".into(), output: "Z".into(), layer: "base".into() }];
    let merged = merge_combos(&g, Some(&ov));
    assert_eq!(merged.len(), 2, "不同触发键应共存");
}

#[test]
fn merge_leader_override_by_sequence() {
    let g = vec![LeaderSequence { keys: vec!["a".into(), "b".into()], output: "OLD".into(), timeout_ms: 0 }];
    let ov = vec![LeaderSequence { keys: vec!["a".into(), "b".into()], output: "NEW".into(), timeout_ms: 0 }];
    let merged = merge_leader(&g, Some(&ov));
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].output, "NEW");
}

#[test]
fn merge_layers_override_base_keyentry() {
    let mut base = HashMap::new();
    base.insert("x".into(), KeyEntry { tap: "{x}".into(), ..Default::default() });
    let merged = merge_layers(
        &anykey_engine::config::Layers {
            layer_maps: [("base".into(), base)].into_iter().collect(),
            hold_term: 150, double_tap_term: 250, double_hold_term: 150,
        },
        Some(&{
            let mut ov_base = HashMap::new();
            ov_base.insert("x".into(), KeyEntry {
                tap: "{x}".into(), hold: "{fn1}".into(), ht: "200".into(), dt: "{y}".into(),
                ..Default::default()
            });
            [("base".into(), ov_base)].into_iter().collect()
        }),
    );
    let e = merged.base().get("x").unwrap();
    assert_eq!(e.hold, "{fn1}", "设备 base 层 x 的 hold 应覆盖");
    assert_eq!(e.dt, "{y}", "设备 base 层 x 的 dt 应覆盖");
}

#[test]
fn multi_device_uses_override_bucket() {
    let mut cfg = base_cfg();
    let mut ov = DeviceOverride::default();
    ov.combo_map = Some(vec![ComboRow { key1: "a".into(), key2: "b".into(), output: "Y".into(), layer: "base".into() }]);
    cfg.devices = [("G1".into(), ov)].into_iter().collect();
    cfg.subscribed_devices = vec![DeviceInfo {
        vid: String::new(), pid: String::new(), alias: String::new(),
        guid: Some("G1".into()), runtime_device_id: Some(1),
        kind: "keyboard".into(), domain_id: None, hardware_id: None, enabled: true,
    }];
    let (ctxs, _d) = build_multi_device_contexts(&cfg, &[1], &[1].into_iter().collect());
    let c = &ctxs[&1].mapping.combo;
    assert_eq!(c.map.get("base").and_then(|m| m.get("a")).and_then(|m| m.get("b")),
        Some(&"{Y}".to_string()), "设备覆盖桶应生效");
}

#[test]
fn multi_device_no_override_inherits_global() {
    let mut cfg = base_cfg();
    cfg.subscribed_devices = vec![DeviceInfo {
        vid: String::new(), pid: String::new(), alias: String::new(),
        guid: Some("G2".into()), runtime_device_id: Some(1),
        kind: "keyboard".into(), domain_id: None, hardware_id: None, enabled: true,
    }];
    let (ctxs, _d) = build_multi_device_contexts(&cfg, &[1], &[1].into_iter().collect());
    let c = &ctxs[&1].mapping.combo;
    assert_eq!(c.map.get("base").and_then(|m| m.get("a")).and_then(|m| m.get("b")),
        Some(&"{X}".to_string()), "无覆盖桶应继承全局");
}

#[test]
fn app_override_merge_chain_device_app_exclusive() {
    let mut cfg = base_cfg();
    cfg.app_aware.apps.insert("chrome.exe".into(), AppOverride {
        combo_map: Some(vec![ComboRow { key1: "a".into(), key2: "b".into(), output: "G".into(), layer: "base".into() }]),
        ..Default::default()
    });
    let mut dev_ov = DeviceOverride::default();
    dev_ov.apps = Some({
        let mut m = HashMap::new();
        m.insert("chrome.exe".into(), AppOverride {
            combo_map: Some(vec![ComboRow { key1: "a".into(), key2: "b".into(), output: "D1".into(), layer: "base".into() }]),
            ..Default::default()
        });
        m
    });
    cfg.devices.insert("G1".into(), dev_ov);
    cfg.subscribed_devices = vec![
        DeviceInfo { vid: "".into(), pid: "".into(), alias: "".into(), guid: Some("G1".into()),
            runtime_device_id: Some(1), kind: "keyboard".into(), domain_id: None, hardware_id: None, enabled: true },
        DeviceInfo { vid: "".into(), pid: "".into(), alias: "".into(), guid: Some("G2".into()),
            runtime_device_id: Some(2), kind: "keyboard".into(), domain_id: None, hardware_id: None, enabled: true },
    ];

    let mapping1 = apply_app_override(&cfg, 1, "chrome.exe",
        &DeviceMapping { tap_dance: HashMap::new(), combo: ComboIndex::default(), leader: LeaderDef { sequences: vec![], timeout_ms: 1000 } });
    let out1 = mapping1.combo.map.get("base").and_then(|m| m.get("a")).and_then(|m| m.get("b"));
    assert_eq!(out1, Some(&"D1".to_string()), "dev1+chrome 应使用设备专属覆盖");

    let mapping2 = apply_app_override(&cfg, 2, "chrome.exe",
        &DeviceMapping { tap_dance: HashMap::new(), combo: ComboIndex::default(), leader: LeaderDef { sequences: vec![], timeout_ms: 1000 } });
    let out2 = mapping2.combo.map.get("base").and_then(|m| m.get("a")).and_then(|m| m.get("b"));
    assert_eq!(out2, Some(&"{G}".to_string()), "dev2+chrome 应使用全局 app fallback");

    let (ctxs, _) = build_multi_device_contexts(&cfg, &[1], &[1].into_iter().collect());
    let out3 = ctxs[&1].mapping.combo.map.get("base").and_then(|m| m.get("a")).and_then(|m| m.get("b"));
    assert_eq!(out3, Some(&"{X}".to_string()), "dev1 无 app 应继承全局 combo");
}

#[test]
fn app_override_json_combo_map_deserialized() {
    // 回归：AppOverride.combo_map 必须有 #[serde(rename_all="camelCase")]
    // JSON 使用 camelCase 的 "comboMap"，缺少 rename 会导致 serde 查找
    // snake_case 的 "combo_map" 失败 → default 为 None → 合并静默丢失。
    let json = r#"{
        "comboMap": [
            {"key1": "{LButton}", "key2": "{RButton}", "output": "{mbutton}", "layer": "base"}
        ],
        "tapDance": {
            "base": {
                "f1": {"tap": "ZZ", "hold": "{f1}"}
            }
        }
    }"#;
    let ov: AppOverride = serde_json::from_str(json).unwrap();
    let cm = ov.combo_map.expect("combo_map 应为 Some，非 None");
    assert_eq!(cm.len(), 1);
    assert_eq!(cm[0].key1, "{LButton}");
    assert_eq!(cm[0].key2, "{RButton}");
    assert_eq!(cm[0].output, "{mbutton}");
    assert_eq!(cm[0].layer, "base");
    let layers = ov.layers.expect("layers 应为 Some");
    let f1 = layers.get("base").and_then(|m| m.get("f1")).unwrap();
    assert_eq!(f1.tap, "ZZ");
    assert_eq!(f1.hold, "{f1}");
}

#[test]
fn app_override_case_insensitive() {
    let mut cfg = base_cfg();
    let mut apps = HashMap::new();
    let mut ov = AppOverride::default();
    let mut ov_base = HashMap::new();
    ov_base.insert("1".into(), KeyEntry {
        tap: "{b}".into(), hold: "{f}".into(), dt: "{g}".into(), dh: "{r}".into(),
        ..Default::default()
    });
    let mut ov_map = HashMap::new();
    ov_map.insert("base".into(), ov_base);
    ov.layers = Some(ov_map);
    apps.insert("revit.exe".into(), ov);
    cfg.app_aware.apps = apps;

    let (contexts, _) = build_multi_device_contexts(&cfg, &[1], &[1].into_iter().collect());
    let dc = contexts.get(&1).unwrap();
    let m = apply_app_override(&cfg, 1, "Revit.exe", &dc.mapping);
    let e = m.tap_dance.get("1").and_then(|lm| lm.get("base")).unwrap();
    assert_eq!(e.tap, "{b}", "Revit.exe 应命中 revit.exe 的 app 覆盖");
    assert_eq!(e.hold, "{f}");
    assert_eq!(e.double_tap, "{g}");
    assert_eq!(e.double_hold, "{r}");
}
