# Clean config.rs with exact replacements

with open('src/config.rs', 'r', encoding='utf-8') as f:
    c = f.read()

# A. Remove tap_dance + flow_control from Config struct
c = c.replace(
    '    #[serde(default)]\n    pub tap_dance: TapDanceConfig,\n\n    #[serde(default)]\n    pub flow_control: FlowControl,\n',
    '')

# B. Delete FlowControl struct + Default impl (exact text from backup)
flow_control_text = '''pub struct FlowControl {
    #[serde(default)]
    pub combo_to_layer: bool,
    #[serde(default)]
    pub combo_to_tap_dance: bool,
}

impl Default for FlowControl {
    fn default() -> Self {
        Self { combo_to_layer: true, combo_to_tap_dance: true }
    }
}
'''
c = c.replace(flow_control_text, '')

# C. Delete TapDanceConfig struct + TapDanceEntry + all default_ht/dt/dh + impl Default
# Find start and end of TapDanceConfig block
td_start = c.find('pub struct TapDanceConfig')
td_end = c.find('fn default_combo_time', td_start)
if td_start >= 0 and td_end >= 0:
    c = c[:td_start] + c[td_end:]

# D. Layers: remove enabled + switch_keys; add timing
old_layers = 'pub struct Layers {\n    #[serde(default)]\n    pub enabled: bool,\n\n    #[serde(default)]\n    pub base_layer: HashMap<String, KeyEntry>,\n\n    #[serde(default)]\n    pub switch_keys: Vec<String>,\n\n    #[serde(default)]\n    pub layer_defs: Vec<LayerDef>,\n}'
new_layers = 'pub struct Layers {\n    #[serde(default)]\n    pub base_layer: HashMap<String, KeyEntry>,\n\n    #[serde(default)]\n    pub layer_defs: Vec<LayerDef>,\n\n    #[serde(default = "default_ht")]\n    pub hold_term: u64,\n\n    #[serde(default = "default_dt")]\n    pub double_tap_term: u64,\n\n    #[serde(default = "default_dh")]\n    pub double_hold_term: u64,\n}'
c = c.replace(old_layers, new_layers)

old_layers_default = 'impl Default for Layers {\n    fn default() -> Self {\n        Self {\n            enabled: true,\n            base_layer: HashMap::new(),\n            switch_keys: vec![],\n            layer_defs: vec![],\n        }\n    }\n}'
new_layers_default = 'impl Default for Layers {\n    fn default() -> Self {\n        Self {\n            base_layer: HashMap::new(),\n            layer_defs: vec![],\n            hold_term: default_ht(),\n            double_tap_term: default_dt(),\n            double_hold_term: default_dh(),\n        }\n    }\n}'
c = c.replace(old_layers_default, new_layers_default)

# E. Add default timing functions
c = c.replace(
    'fn default_combo_time() -> u64 { 200 }',
    'fn default_combo_time() -> u64 { 200 }\nfn default_ht() -> u64 { 150 }\nfn default_dt() -> u64 { 250 }\nfn default_dh() -> u64 { 150 }')

# F. Clean blanks
c = c.replace('\n\n\n', '\n\n')

with open('src/config.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('config.rs done')
