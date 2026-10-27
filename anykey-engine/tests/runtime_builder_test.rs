//! runtime_builder 单元测试（从 src/runtime_builder.rs 的 #[cfg(test)] 模块分离）。
//! 作为集成测试放在 tests/ 目录，仅可访问 pub API。

use anykey_engine::config::{
    AppAwareConfig, AppOverride, ComboRow, Config, DeviceInfo, DeviceOverride, KeyEntry,
    LeaderConfig, LeaderSequence, Layers,
};
use anykey_engine::util::is_unmap_output;
use anykey_engine::state::{ComboIndex, DeviceMapping, KeyDef, LeaderDef};
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

// ══════════════════════════════════════════════════════════════
// {unmap}：覆盖层的「删除本层条目」标记（docs/design_unmap_override.md）
// ══════════════════════════════════════════════════════════════

/// 全局配置：a+s@base → {left}、a+s@fn2 → {home}（同一对键在两层各有条目）；
/// CapsLock 点按 {esc} / 按住切 fn1；leader q,w → {esc}。
fn cfg_with_global() -> Config {
    let mut base = HashMap::new();
    base.insert("capslock".to_string(), KeyEntry {
        tap: "{esc}".into(), hold: "{fn1}".into(), ht: "150".into(), ..Default::default()
    });
    Config {
        combo_time: 200,
        combo_map: vec![
            ComboRow { key1: "a".into(), key2: "s".into(), output: "{left}".into(), layer: "base".into() },
            ComboRow { key1: "a".into(), key2: "s".into(), output: "{home}".into(), layer: "fn2".into() },
        ],
        layers: Layers {
            layer_maps: [("base".to_string(), base)].into_iter().collect(),
            hold_term: 150, double_tap_term: 250, double_hold_term: 150,
        },
        leader: LeaderConfig {
            timeout_ms: 2000,
            sequences: vec![LeaderSequence {
                keys: vec!["q".into(), "w".into()], output: "{esc}".into(), timeout_ms: 0,
            }],
            loop_capture: false,
        },
        subscribed_devices: vec![],
        devices: HashMap::new(),
        app_aware: AppAwareConfig::default(),
        per_device: false,
    }
}

fn with_app(mut cfg: Config, app: &str, ov: AppOverride) -> Config {
    cfg.app_aware.apps.insert(app.to_string(), ov);
    cfg
}

fn combo_has(m: &DeviceMapping, layer: &str, key: &str) -> bool {
    m.combo.map.get(layer).map_or(false, |km| km.contains_key(key))
}

fn td_field(m: &DeviceMapping, phys: &str, layer: &str) -> Option<KeyDef> {
    m.tap_dance.get(phys).and_then(|lm| lm.get(layer)).cloned()
}

#[test]
fn unmap_clears_combo_only_in_named_layer() {
    let cfg = with_app(cfg_with_global(), "streetfighter6.exe", AppOverride {
        combo_map: Some(vec![ComboRow {
            key1: "a".into(), key2: "s".into(), output: "{unmap}".into(), layer: "base".into(),
        }]),
        ..Default::default()
    });

    let dev = build_default_runtime(&cfg);
    assert!(combo_has(&dev, "base", "a"), "前提：全局 base 层有 a+s");
    assert!(combo_has(&dev, "fn2", "a"), "前提：全局 fn2 层也有 a+s");

    let m = apply_app_override(&cfg, 1, "streetfighter6.exe", &dev);
    assert!(!combo_has(&m, "base", "a"), "base 层 a+s 应被 {{unmap}} 清除");
    assert!(combo_has(&m, "fn2", "a"), "只清 base 层，fn2 的同键对应保留");
    assert!(m.leader.sequences.len() == 1, "未涉及的 leader 序列不受影响");
}

#[test]
fn unmap_clears_single_td_field() {
    let mut ov_base = HashMap::new();
    ov_base.insert("capslock".to_string(), KeyEntry {
        tap: "{esc}".into(), hold: "{unmap}".into(), ..Default::default()
    });
    let cfg = with_app(cfg_with_global(), "sf6.exe", AppOverride {
        layers: Some([("base".to_string(), ov_base)].into_iter().collect()),
        ..Default::default()
    });

    let m = apply_app_override(&cfg, 1, "sf6.exe", &build_default_runtime(&cfg));
    let e = td_field(&m, "capslock", "base").expect("tap 仍在 → 条目应保留");
    assert_eq!(e.tap, "{esc}", "未动的字段保持覆盖行自己的值");
    assert_eq!(e.hold, "", "hold 应被 {{unmap}} 置空");
    assert!(!e.has_td(), "无 hold/dt/dh → 退化为纯点按重映射");
}

#[test]
fn unmap_clears_whole_td_entry_when_all_fields_empty() {
    let mut ov_base = HashMap::new();
    ov_base.insert("capslock".to_string(), KeyEntry {
        tap: "{unmap}".into(), hold: "{unmap}".into(),
        dt: "{unmap}".into(), dh: "{unmap}".into(), ht: "150".into(),
        ..Default::default()
    });
    let cfg = with_app(cfg_with_global(), "sf6.exe", AppOverride {
        layers: Some([("base".to_string(), ov_base)].into_iter().collect()),
        ..Default::default()
    });

    let m = apply_app_override(&cfg, 1, "sf6.exe", &build_default_runtime(&cfg));
    assert!(!m.tap_dance.contains_key("capslock"),
            "四字段全空 → 整条不存在（ht 也随之失效）");
}

#[test]
fn unmap_clears_leader_sequence() {
    let cfg = with_app(cfg_with_global(), "sf6.exe", AppOverride {
        leader: Some(vec![LeaderSequence {
            keys: vec!["q".into(), "w".into()], output: "{unmap}".into(), timeout_ms: 0,
        }]),
        ..Default::default()
    });

    let m = apply_app_override(&cfg, 1, "sf6.exe", &build_default_runtime(&cfg));
    assert!(m.leader.sequences.is_empty(), "被 {{unmap}} 的序列不应存在");
}

#[test]
fn unmap_in_inner_layer_can_be_added_back() {
    // 外层（应用）清掉 a+s@base，内层（设备）同 identity 重新给出 → 内层优先
    let mut cfg = with_app(cfg_with_global(), "sf6.exe", AppOverride {
        combo_map: Some(vec![ComboRow {
            key1: "a".into(), key2: "s".into(), output: "{unmap}".into(), layer: "base".into(),
        }]),
        ..Default::default()
    });
    cfg.subscribed_devices.push(DeviceInfo {
        vid: "1234".into(), pid: "5678".into(), alias: "kbd".into(),
        guid: Some("DEV-A".into()), runtime_device_id: Some(7), hardware_id: None,
        kind: "keyboard".into(), domain_id: None, enabled: true,
    });
    cfg.devices.insert("DEV-A".into(), DeviceOverride {
        combo_map: Some(vec![ComboRow {
            key1: "a".into(), key2: "s".into(), output: "{right}".into(), layer: "base".into(),
        }]),
        ..Default::default()
    });

    let (ctxs, _) = build_multi_device_contexts(&cfg, &[7], &[7].into_iter().collect());
    let m = apply_app_override(&cfg, 7, "sf6.exe", &ctxs[&7].mapping);
    assert_eq!(
        m.combo.map.get("base").and_then(|km| km.get("a")).and_then(|p| p.get("s")),
        Some(&"{right}".to_string()),
        "更内层（设备）可以把它加回来 —— 内层优先，与覆盖规则一致"
    );
}

#[test]
fn unmap_in_global_layer_is_dropped_and_warned() {
    let mut cfg = cfg_with_global();
    cfg.combo_map.push(ComboRow {
        key1: "x".into(), key2: "y".into(), output: "{unmap}".into(), layer: "base".into(),
    });
    cfg.layers.layer_maps.get_mut("base").unwrap().get_mut("capslock").unwrap().hold = "{unmap}".into();

    let warns = anykey_engine::config::unmap_warnings(&cfg);
    assert_eq!(warns.len(), 2, "一条 combo + 一条 tapDance 字段，各告警一次: {:?}", warns);
    assert!(warns.iter().any(|w| w.contains("comboMap") && w.contains("x + y")));
    assert!(warns.iter().any(|w| w.contains("tapDance") && w.contains("hold")));

    let m = build_default_runtime(&cfg);
    assert!(!combo_has(&m, "base", "x"), "全局的 {{unmap}} 行按「不存在」处理");
    let e = td_field(&m, "capslock", "base").expect("tap 仍在，条目保留");
    assert_eq!(e.hold, "", "全局的 {{unmap}} 字段按空处理");
    assert_eq!(e.tap, "{esc}", "其余字段不受影响");
    // 最关键的不变量：这个字符串绝不能进 mapping（否则管道会当未知键名静默丢弃）
    for lm in m.tap_dance.values() {
        for e in lm.values() {
            for v in [&e.tap, &e.hold, &e.double_tap, &e.double_hold] {
                assert!(!v.contains("unmap"), "mapping 里不允许残留 {{unmap}}: {}", v);
            }
        }
    }
}

#[test]
fn unmap_applies_when_per_device_disabled() {
    // perDevice=false：设备桶被忽略，但「全局 × app」这条路径仍要生效
    let cfg = with_app(cfg_with_global(), "sf6.exe", AppOverride {
        combo_map: Some(vec![ComboRow {
            key1: "a".into(), key2: "s".into(), output: "{unmap}".into(), layer: "base".into(),
        }]),
        ..Default::default()
    });
    assert!(!cfg.per_device);

    let mut mgr = RuntimeManager::new();
    mgr.build_app_mappings(&cfg, &[1], "sf6.exe");
    let m = mgr.lookup_mapping(1, "sf6.exe").expect("perDevice=false 也应构建全局×app map");
    assert!(!combo_has(&m, "base", "a"));
}

#[test]
fn unmap_token_is_case_insensitive_but_strict() {
    assert!(is_unmap_output("{unmap}"));
    assert!(is_unmap_output("{UNMAP}"));
    assert!(is_unmap_output("  {Unmap}\t"));
    assert!(!is_unmap_output("unmap"), "裸写不认（会被当文本输出）");
    assert!(!is_unmap_output("{unmap }"), "内部空格不认");
    assert!(!is_unmap_output("{unmapped}"));
    assert!(!is_unmap_output(""));
}
