import re, glob

base = '.'

# ── state.rs ──
with open(f'{base}/src/state.rs', 'r', encoding='utf-8') as f:
    c = f.read()

# Remove tap_dance_cfg + combo_time_ms from DeviceMapping
old_dm = '''pub struct DeviceMapping {
    pub tap_dance:     HashMap<String, HashMap<String, KeyDef>>,
    pub combo:         ComboIndex,
    pub leader:        LeaderDef,
    pub tap_dance_cfg: crate::config::TapDanceConfig,
    pub combo_time_ms: u64,
}'''
new_dm = '''pub struct DeviceMapping {
    pub tap_dance: HashMap<String, HashMap<String, KeyDef>>,
    pub combo:     ComboIndex,
    pub leader:    LeaderDef,
}'''
c = c.replace(old_dm, new_dm)

# Change timing reads from mapping back to config.layers
c = c.replace('self.runtime.mapping.tap_dance_cfg.hold_term', 'self.config.layers.hold_term')
c = c.replace('self.runtime.mapping.tap_dance_cfg.double_tap_term', 'self.config.layers.double_tap_term')
c = c.replace('self.runtime.mapping.tap_dance_cfg.double_hold_term', 'self.config.layers.double_hold_term')
c = c.replace('self.runtime.mapping.combo_time_ms', 'self.config.combo_time')

with open(f'{base}/src/state.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('state.rs done')

# ── runtime_builder.rs ──
with open(f'{base}/src/runtime_builder.rs', 'r', encoding='utf-8') as f:
    c = f.read()

# Remove tap_dance_cfg: and combo_time_ms: from all DeviceMapping inits
c = re.sub(r',\n\s+tap_dance_cfg: [^,\n]+,?\n?\s+combo_time_ms: [^,\n]+,?', '', c)
c = re.sub(r',\n\s+combo_time_ms: [^,\n]+,?', '', c)
c = re.sub(r',\n\s+tap_dance_cfg: [^,\n]+,?', '', c)
# Handle trailing comma in DeviceMapping { ..., }
# The regex might leave a trailing comma before }
c = re.sub(r',\n    \}', '\n    }', c)

# Fix apply_app_override - remove td_cfg and combo_time_ms
c = c.replace(
    "    let td_cfg = if let Some(ref td) = app_ov.tap_dance {\n        let base = &config.tap_dance;\n        crate::config::TapDanceConfig {\n            enabled:         td.enabled,\n            hold_term:       if td.hold_term > 0 { td.hold_term } else { base.hold_term },\n            double_tap_term: if td.double_tap_term > 0 { td.double_tap_term } else { base.double_tap_term },\n            double_hold_term: if td.double_hold_term > 0 { td.double_hold_term } else { base.double_hold_term },\n            keys:            if !td.keys.is_empty() { td.keys.clone() } else { base.keys.clone() },\n        }\n    } else {\n        config.tap_dance.clone()\n    };",
    '')
c = c.replace(
    "DeviceMapping {\n        tap_dance:     build_tap_dance_map(&config.layers, &merged_combos),\n        combo:         build_combo_index(&merged_combos),\n        leader:        LeaderDef { sequences: merged_leader, timeout_ms: config.leader.timeout_ms },\n        tap_dance_cfg: td_cfg,\n        combo_time_ms: app_ov.combo_time.unwrap_or(config.combo_time),\n    }",
    "DeviceMapping {\n        tap_dance: build_tap_dance_map(&config.layers, &merged_combos),\n        combo:     build_combo_index(&merged_combos),\n        leader:    LeaderDef { sequences: merged_leader, timeout_ms: config.leader.timeout_ms },\n    }")

with open(f'{base}/src/runtime_builder.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('runtime_builder.rs done')

# ── pipeline/combo.rs ──
for fpath in [f'{base}/src/pipeline/combo.rs', f'{base}/src/pipeline.rs']:
    try:
        with open(fpath, 'r', encoding='utf-8') as f:
            c = f.read()
        c = c.replace('self.runtime.mapping.combo_time_ms', 'self.config.combo_time')
        c = c.replace('self.config.flow_control.combo_to_layer', 'true /* always skip layer for combo */')
        c = c.replace('self.config.flow_control.combo_to_tap_dance', 'false')
        with open(fpath, 'w', encoding='utf-8') as f:
            f.write(c)
        print(f'{fpath} done')
    except FileNotFoundError:
        pass

# ── Tests ──
for fpath in glob.glob(f'{base}/tests/*.rs'):
    with open(fpath, 'r', encoding='utf-8') as f:
        c = f.read()
    # Remove flow_control from test configs
    c = re.sub(r',?\n\s+flow_control: FlowControl \{.*?\n\s+\},?', '', c)
    c = re.sub(r'flow_control: crate::config::FlowControl::default\(\),?\n', '', c)
    c = re.sub(r'use.*FlowControl;\n', '', c)
    # Remove tap_dance from test configs
    c = re.sub(r',?\n\s+tap_dance: TapDanceConfig \{.*?\n\s+\},?', '', c, flags=re.DOTALL)
    c = re.sub(r'use.*TapDanceConfig;\n', '', c)
    # Remove switch_keys
    c = c.replace('switch_keys: vec![],', '')
    c = c.replace('switch_keys: vec![],\n', '')
    # Add timing to Layers in test configs (after layer_defs)
    c = re.sub(
        r'(layer_defs: vec!\[\],?)',
        r'\1\n            hold_term: 150,\n            double_tap_term: 250,\n            double_hold_term: 150,',
        c)
    # Clean up imports
    c = re.sub(r'use.*TapDanceConfig;\n', '', c)
    c = re.sub(r'use.*FlowControl;\n', '', c)
    with open(fpath, 'w', encoding='utf-8') as f:
        f.write(c)
    print(f'test {fpath} done')

print('\nAll done')
