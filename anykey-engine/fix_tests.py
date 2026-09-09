import os, re

for fpath in os.listdir('tests'):
    if not fpath.endswith('.rs'):
        continue
    fpath = f'tests/{fpath}'
    with open(fpath, 'r', encoding='utf-8') as f:
        c = f.read()
    c = c.replace('use anykey_engine::runtime_builder::build_default_runtime;', '')
    c = c.replace('use anykey_engine::runtime_builder::{build_default_runtime};', '')
    c = c.replace('.resolve(', '.get_mapping(')
    c = c.replace('use anykey_engine::runtime_builder::RuntimeManager;', '')
    c = c.replace('use anykey_engine::state::Runtime;', '')
    c = re.sub(r',\s*Runtime\b', '', c)
    c = re.sub(r'\bRuntime\b,\s*', '', c)
    c = c.replace('pipeline.current_key = (', '// v3: current_key = (')
    c = c.replace('pipeline.current_key', 'pipeline.current_device // v3: was current_key')
    c = c.replace('let active_key = &pipeline.current_key;', 'let active_key = &(pipeline.current_device, pipeline.current_app.clone());')
    c = c.replace('// v3: use pipeline.mapping / pipeline.state.runtime', '')
    c = c.replace('// v3: use pipeline.mapping / pipeline.state.', '')
    with open(fpath, 'w', encoding='utf-8') as f:
        f.write(c)
    print(f'  {fpath}')
print('done')
