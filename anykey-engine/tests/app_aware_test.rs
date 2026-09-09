//! 应用感知 + 多设备隔离测试（v3: mapping 隔离=(device,app)，state 隔离=domain）
//! Domain 隔离依赖 domain_id 字段；同 domain 设备共享 state。

use anykey_engine::config::{AppAwareConfig, AppOverride, ComboRow, Config, Layers, LeaderConfig, KeyEntry, DeviceInfo};
use anykey_engine::state::{PipelineState, EmitEvent};
use anykey_engine::runtime_builder::{self, RuntimeManager};
use std::collections::HashMap;
use std::sync::Arc;

fn make_config() -> Config {
    Config {
        combo_time: 200,
        combo_map: vec![],
        layers: Layers {
            layer_maps: [("base".into(), {
                let mut m = HashMap::new();
                m.insert("a".into(), KeyEntry { tap: "{X}".into(), ..Default::default() });
                m
            })].into_iter().collect(),
            hold_term: 150,
            double_tap_term: 250,
            double_hold_term: 150,
        },
        leader: LeaderConfig::default(),
        subscribed_devices: vec![],
        devices: HashMap::new(),
        app_aware: AppAwareConfig {
            apps: {
                let mut m = HashMap::new();
                m.insert("chrome.exe".into(), AppOverride {
                    combo_map: Some(vec![
                        ComboRow { key1: "j".into(), key2: "k".into(), output: "Z".into(), layer: "base".into() },
                    ]),
                    ..Default::default()
                });
                m
            },
        },
        per_device: false,
    }
}

/// PipelineState::new 从 config 构建全量映射——不再依赖 RuntimeManager。
#[test]
fn test_unconfigured_app_uses_default_runtime() {
    let config = make_config();
    let mut pipeline = PipelineState::new(config.clone());

    pipeline.key_down("a");
    pipeline.advance_ms(160);
    pipeline.key_up("a");

    let has_x = pipeline.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "X"));
    assert!(has_x, "默认 mapping 应使用 config 中 a→X");
}

/// RuntimeManager 按 (device, app) 构建 Arc<DeviceMapping>——chrome 的 combo 覆盖生效。
#[test]
fn test_configured_app_has_independent_runtime() {
    let config = make_config();
    let mut pipeline = PipelineState::new(config.clone());
    let mut mgr = RuntimeManager::new();

    // 加载 chrome.exe 的 mapping
    pipeline.mapping = mgr.get_mapping(&config, 1, "chrome.exe");
    pipeline.current_device = 1;
    pipeline.current_app = "chrome.exe".into();

    pipeline.key_down("j");
    pipeline.advance_ms(10);
    pipeline.key_down("k");
    pipeline.advance_ms(config.combo_time + 10);

    let has_z = pipeline.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "Z"));
    assert!(has_z, "chrome 的 combo j+k 应触发 Z");

    // 切回默认 mapping
    pipeline.mapping = Arc::new(build_default_mapping(&config));
    pipeline.current_app = String::new();
    pipeline.emit_log.clear();

    pipeline.key_down("a");
    pipeline.advance_ms(160);
    pipeline.key_up("a");
    let has_x_after = pipeline.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "X"));
    assert!(has_x_after, "切回默认后 a→X 仍有效");
}

/// 不同 domain 的设备状态隔离
#[test]
fn test_device_switch_preserves_key_state() {
    let mut config = make_config();
    // 设备 1 domain=1, 设备 2 domain=2
    config.subscribed_devices = vec![
        DeviceInfo { vid: "".into(), pid: "".into(), alias: "".into(), guid: Some("g1".into()),
            runtime_device_id: Some(1), kind: "keyboard".into(), domain_id: Some(1), hardware_id: None, enabled: true },
        DeviceInfo { vid: "".into(), pid: "".into(), alias: "".into(), guid: Some("g2".into()),
            runtime_device_id: Some(2), kind: "keyboard".into(), domain_id: Some(2), hardware_id: None, enabled: true },
    ];

    let mut pipeline = PipelineState::new(config.clone());
    let mut mgr = RuntimeManager::new();

    // 设备 1 domain=1: 按下 a
    pipeline.mapping = mgr.get_mapping(&config, 1, "");
    pipeline.state = mgr.load_domain_state(1);
    pipeline.current_domain = 1;
    pipeline.current_device = 1;
    pipeline.key_down("a");
    assert!(pipeline.state.keys.key_states.contains_key("a"));
    mgr.save_domain_state(1, pipeline.state.clone());

    // 切设备 2 domain=2: 没 a
    let state2 = mgr.load_domain_state(2);
    assert!(!state2.keys.key_states.contains_key("a"), "设备 2 domain=2 独立，不应有 a");

    // 切回设备 1: a 还在
    pipeline.state = mgr.load_domain_state(1);
    assert!(pipeline.state.keys.key_states.contains_key("a"), "切回 domain=1 后 a 状态还在");
}

/// v3: 同一 domain 内键鼠共享 state——各自 mapping 独立
#[test]
fn test_simultaneous_keys_across_devices() {
    let mut config = make_config();
    config.subscribed_devices = vec![
        DeviceInfo { vid: "".into(), pid: "".into(), alias: "".into(), guid: Some("g1".into()),
            runtime_device_id: Some(1), kind: "keyboard".into(), domain_id: Some(1), hardware_id: None, enabled: true },
        DeviceInfo { vid: "".into(), pid: "".into(), alias: "".into(), guid: Some("g2".into()),
            runtime_device_id: Some(2), kind: "mouse".into(), domain_id: Some(1), hardware_id: None, enabled: true },
    ];

    let mut pipeline = PipelineState::new(config.clone());
    let mut mgr = RuntimeManager::new();
    pipeline.state = mgr.load_domain_state(1);
    pipeline.current_domain = 1;

    // 键盘按 a
    pipeline.mapping = mgr.get_mapping(&config, 1, "");
    pipeline.current_device = 1;
    pipeline.key_down("a");
    assert!(pipeline.state.keys.key_states.contains_key("a"), "键盘按 a 后 state 应有 a");

    // 同 domain 鼠标按 b——state 共享，能看到 a
    pipeline.mapping = mgr.get_mapping(&config, 2, "");
    pipeline.current_device = 2;
    assert!(pipeline.state.keys.key_states.contains_key("a"), "同 domain 键鼠共享 state");
    pipeline.key_down("b");

    // 抬起清理
    pipeline.key_up("a");
    pipeline.key_up("b");
    assert!(!pipeline.state.keys.key_states.contains_key("a"));
    assert!(!pipeline.state.keys.key_states.contains_key("b"));
}

/// 应用覆盖生效：chrome 的 combo j+k → Z
#[test]
fn test_app_override_combo_works() {
    let config = make_config();
    let mut pipeline = PipelineState::new(config.clone());
    let mut mgr = RuntimeManager::new();

    pipeline.mapping = mgr.get_mapping(&config, 1, "chrome.exe");
    pipeline.current_device = 1;
    pipeline.current_app = "chrome.exe".into();

    pipeline.key_down("j");
    pipeline.advance_ms(10);
    pipeline.key_down("k");
    pipeline.advance_ms(config.combo_time + 10);

    let has_z = pipeline.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == "Z"));
    assert!(has_z, "combo j+k 应该在 chrome 环境触发 Z");
}

/// 同一应用不同设备：mapping 独立，state 共享（同 domain）
#[test]
fn test_same_app_different_devices_state_isolated() {
    let mut config = make_config();
    config.subscribed_devices = vec![
        DeviceInfo { vid: "".into(), pid: "".into(), alias: "".into(), guid: Some("g1".into()),
            runtime_device_id: Some(1), kind: "keyboard".into(), domain_id: Some(1), hardware_id: None, enabled: true },
        DeviceInfo { vid: "".into(), pid: "".into(), alias: "".into(), guid: Some("g2".into()),
            runtime_device_id: Some(3), kind: "mouse".into(), domain_id: Some(1), hardware_id: None, enabled: true },
    ];

    let mut pipeline = PipelineState::new(config.clone());
    let mut mgr = RuntimeManager::new();
    pipeline.state = mgr.load_domain_state(1);
    pipeline.current_domain = 1;

    // 键盘按 a
    pipeline.mapping = mgr.get_mapping(&config, 1, "");
    pipeline.current_device = 1;
    pipeline.key_down("a");
    pipeline.advance_ms(160);
    pipeline.key_up("a");

    // 鼠标按 b（同 domain，state 共享，a 已清）
    pipeline.mapping = mgr.get_mapping(&config, 3, "");
    pipeline.current_device = 3;
    pipeline.key_down("b");
    pipeline.key_up("b");

    assert!(!pipeline.state.keys.key_states.contains_key("a"));
    assert!(!pipeline.state.keys.key_states.contains_key("b"));
}

// ── Helpers ──

fn build_default_mapping(config: &Config) -> anykey_engine::state::DeviceMapping {
    let (ctxs, _) = runtime_builder::build_default_device_context(config);
    ctxs[&1].mapping.clone()
}
