import os, re

base = '.'

# ── 1. AppOverride: add layers field ──
with open(f'{base}/src/config.rs', 'r', encoding='utf-8') as f:
    c = f.read()
old_app = '#[derive(Debug, Clone, Default, serde::Deserialize)]\npub struct AppOverride {\n    #[serde(default)]\n    pub combo_time: Option<u64>,\n    #[serde(default)]\n    pub combo_map:  Option<Vec<ComboRow>>,\n    #[serde(default)]\n    pub leader:     Option<Vec<crate::config::LeaderSequence>>,\n}'
new_app = '#[derive(Debug, Clone, Default, serde::Deserialize)]\npub struct AppOverride {\n    #[serde(default)]\n    pub combo_time: Option<u64>,\n    #[serde(default)]\n    pub combo_map:  Option<Vec<ComboRow>>,\n    #[serde(default)]\n    pub leader:     Option<Vec<crate::config::LeaderSequence>>,\n    #[serde(default)]\n    pub layers:     Option<HashMap<String, HashMap<String, KeyEntry>>>,\n}'
c = c.replace(old_app, new_app)
with open(f'{base}/src/config.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('config.rs: AppOverride +layers')

# ── 2. Rename tap_dance → layers_map in all Rust source ──
patterns = [
    ('pub tap_dance:', 'pub layers_map:'),
    ('.tap_dance.get(', '.layers_map.get('),
    ('.tap_dance[', '.layers_map['),
    ('self.runtime.mapping.tap_dance', 'self.runtime.mapping.layers_map'),
    ('fn build_tap_dance_map', 'fn build_layers_map'),
    ('build_tap_dance_map(', 'build_layers_map('),
    ('tap_dance_map', 'layers_map'),
    ('global_td', 'global_lm'),
    ('tap_dance: build_layers_map', 'layers_map: build_layers_map'),
    ('tap_dance: HashMap::new()', 'layers_map: HashMap::new()'),
    ('tap_dance: global_lm.clone()', 'layers_map: global_lm.clone()'),
    ('tap_dance: layers_map', 'layers_map: layers_map'),
]

for fpath in ['src/state.rs', 'src/runtime_builder.rs', 'src/pipeline.rs',
              'src/pipeline/combo.rs', 'src/pipeline/defer.rs',
              'src/pipeline/tap_dance.rs', 'src/pipeline/commit.rs',
              'src/pipeline/leader.rs']:
    fpath = f'{base}/{fpath}'
    if not os.path.exists(fpath):
        continue
    with open(fpath, 'r', encoding='utf-8') as f:
        c = f.read()
    for old, new in patterns:
        c = c.replace(old, new)
    with open(fpath, 'w', encoding='utf-8') as f:
        f.write(c)

print('src files: tap_dance → layers_map')

# ── 3. Tests ──
for fpath in os.listdir(f'{base}/tests'):
    if not fpath.endswith('.rs'):
        continue
    fpath = f'{base}/tests/{fpath}'
    with open(fpath, 'r', encoding='utf-8') as f:
        c = f.read()
    for old, new in patterns:
        c = c.replace(old, new)
    with open(fpath, 'w', encoding='utf-8') as f:
        f.write(c)

print('tests done')

# ── 4. apply_app_override: use merged layers ──
with open(f'{base}/src/runtime_builder.rs', 'r', encoding='utf-8') as f:
    c = f.read()

old_apply = 'fn apply_app_override(config: &Config, app: &str, dev_mapping: &DeviceMapping) -> DeviceMapping {\n    let app_ov = match config.app_aware.apps.get(app) {\n        Some(ov) => ov,\n        None => return dev_mapping.clone()\n    };\n    let merged_combos = if let Some(ref rows) = app_ov.combo_map {\n        merge_combos(&config.combo_map, Some(rows))\n    } else { config.combo_map.clone() };\n\n    let merged_leader = if let Some(ref seqs) = app_ov.leader {\n        merge_leader(&config.leader.sequences, Some(seqs))\n    } else { config.leader.sequences.clone() };\n\n    // tapDance: field-by-field merge with device/global fallback\n    DeviceMapping {\n        layers_map: build_layers_map(&config.layers, &merged_combos),\n        combo:     build_combo_index(&merged_combos),\n        leader:    LeaderDef { sequences: merged_leader, timeout_ms: config.leader.timeout_ms }\n    }\n}'
new_apply = 'fn apply_app_override(config: &Config, app: &str, dev_mapping: &DeviceMapping) -> DeviceMapping {\n    let app_ov = match config.app_aware.apps.get(app) {\n        Some(ov) => ov,\n        None => return dev_mapping.clone()\n    };\n    let merged_layers = merge_layers(&config.layers, app_ov.layers.as_ref());\n    let merged_combos = if let Some(ref rows) = app_ov.combo_map {\n        merge_combos(&config.combo_map, Some(rows))\n    } else { config.combo_map.clone() };\n\n    let merged_leader = if let Some(ref seqs) = app_ov.leader {\n        merge_leader(&config.leader.sequences, Some(seqs))\n    } else { config.leader.sequences.clone() };\n\n    DeviceMapping {\n        layers_map: build_layers_map(&merged_layers, &merged_combos),\n        combo:      build_combo_index(&merged_combos),\n        leader:     LeaderDef { sequences: merged_leader, timeout_ms: config.leader.timeout_ms }\n    }\n}'
c = c.replace(old_apply, new_apply)
with open(f'{base}/src/runtime_builder.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('apply_app_override: +merged_layers')

# ── 5. Verify no tap_dance remaining in source ──
import subprocess
r = subprocess.run(['grep', '-rn', 'tap_dance\\b', 'src/', 'tests/'], capture_output=True, text=True, cwd=base)
lines = [l for l in r.stdout.split('\n') if l and 'backups' not in l]
if lines:
    print(f'WARNING: {len(lines)} tap_dance references remain:')
    for l in lines[:10]:
        print(f'  {l}')
else:
    print('All tap_dance references cleaned')
