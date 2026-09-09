import re

with open('src/config.rs', 'r', encoding='utf-8') as f:
    c = f.read()

# 1. Remove flow_control and tap_dance from Config struct
c = c.replace(
    '    #[serde(default)]\n    pub tap_dance: TapDanceConfig,\n\n    #[serde(default)]\n    pub flow_control: FlowControl,\n',
    '')

# 2. Remove FlowControl struct + Default impl
old_flow = 'pub struct FlowControl {\n    #[serde(default)]\n    pub combo_to_layer: bool,\n    #[serde(default)]\n    pub combo_to_tap_dance: bool,\n}\n\nimpl Default for FlowControl {\n    fn default() -> Self {\n        Self { combo_to_layer: true, combo_to_tap_dance: true }\n    }\n}\n'
c = c.replace(old_flow, '')

# 3. Remove TapDanceConfig + TapDanceEntry + defaults
c = re.sub(r'pub struct TapDanceConfig \{.*?\n\}', '', c, flags=re.DOTALL)
c = re.sub(r'pub struct TapDanceEntry \{.*?\n\}', '', c, flags=re.DOTALL)
c = re.sub(r'fn default_ht.*?\n    \d+\n\}', '', c, flags=re.DOTALL)
c = re.sub(r'fn default_dt.*?\n    \d+\n\}', '', c, flags=re.DOTALL)
c = re.sub(r'fn default_dh.*?\n    \d+\n\}', '', c, flags=re.DOTALL)

# 5. Layers: remove enabled, switch_keys; add timing
old_layers = 'pub struct Layers {\n    #[serde(default)]\n    pub enabled: bool,\n\n    #[serde(default)]\n    pub base_layer: HashMap<String, KeyEntry>,\n\n    #[serde(default)]\n    pub switch_keys: Vec<String>,\n\n    #[serde(default)]\n    pub layer_defs: Vec<LayerDef>,\n}'
new_layers = 'pub struct Layers {\n    #[serde(default)]\n    pub base_layer: HashMap<String, KeyEntry>,\n\n    #[serde(default)]\n    pub layer_defs: Vec<LayerDef>,\n\n    #[serde(default = "default_ht")]\n    pub hold_term: u64,\n\n    #[serde(default = "default_dt")]\n    pub double_tap_term: u64,\n\n    #[serde(default = "default_dh")]\n    pub double_hold_term: u64,\n}'
c = c.replace(old_layers, new_layers)

# 6. Update Layers Default impl
old_default = 'impl Default for Layers {\n    fn default() -> Self {\n        Self {\n            enabled: true,\n            base_layer: HashMap::new(),\n            switch_keys: vec![],\n            layer_defs: vec![],\n        }\n    }\n}'
new_default = 'impl Default for Layers {\n    fn default() -> Self {\n        Self {\n            base_layer: HashMap::new(),\n            layer_defs: vec![],\n            hold_term: default_ht(),\n            double_tap_term: default_dt(),\n            double_hold_term: default_dh(),\n        }\n    }\n}'
c = c.replace(old_default, new_default)

# 7. Restore default timing functions (removed by regex in step 3)
c = c.replace(
    'fn default_combo_time() -> u64 { 200 }',
    'fn default_combo_time() -> u64 { 200 }\nfn default_ht() -> u64 { 150 }\nfn default_dt() -> u64 { 250 }\nfn default_dh() -> u64 { 150 }')

# 8. Clean up blank lines
c = re.sub(r'\n\n\n+', '\n\n', c)

with open('src/config.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('config.rs updated')
