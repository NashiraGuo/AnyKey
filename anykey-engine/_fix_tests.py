"""Fix old Layers format in test files."""
import os, re

files_to_fix = [
    "tests/app_aware_test.rs",
    "tests/leak_test.rs",
    "tests/pipeline_test.rs",
]

for fpath in files_to_fix:
    path = os.path.join(os.path.dirname(__file__) or '.', fpath)
    if not os.path.exists(path):
        print(f"SKIP: {path}")
        continue
    with open(path, 'r') as f:
        content = f.read()

    # Replace struct literal fields
    content = content.replace('base_layer: HashMap::new(),\n            layer_defs: vec![],',
                              'layer_maps: HashMap::new(),')
    content = content.replace('base_layer: HashMap::new(),\n            layer_defs: vec![],',
                              'layer_maps: HashMap::new(),')
    
    # Replace base_layer: { entries }, layer_defs: vec![]
    content = re.sub(
        r'base_layer:\s*(\{[^}]*\}),\s*\n\s*layer_defs:\s*vec!\[\]',
        lambda m: f'layer_maps: [("base".into(), {m.group(1)})].into_iter().collect()',
        content
    )

    # Replace .base_layer.insert( with .layer_maps.entry("base".into()).or_default().insert(
    content = content.replace('.layers.base_layer.insert(', '.layers.layer_maps.entry("base".into()).or_default().insert(')
    
    # Replace layer_defs.push(LayerDef { name: "X".into(), key_map: { ... } })
    content = re.sub(
        r'(\w+)\.layer_defs\.push\(LayerDef\s*\{\s*name:\s*"([^"]+)"\.into\(\),\s*key_map:\s*',
        r'\1.layer_maps.insert("\2".into(), ',
        content
    )
    # Remove the trailing }) from layer_defs.push 
    content = content.replace('LayerDef {', '/* unused LayerDef */ {')
    
    with open(path, 'w') as f:
        f.write(content)
    print(f"FIXED: {path}")

print("done")
