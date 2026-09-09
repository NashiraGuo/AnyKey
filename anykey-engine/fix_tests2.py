import os, re

for fpath in os.listdir('tests'):
    if not fpath.endswith('.rs'):
        continue
    fpath = f'tests/{fpath}'
    with open(fpath, 'r', encoding='utf-8') as f:
        c = f.read()

    # build_default_runtime(cfg) → Arc::new(build_mapping(cfg))
    c = c.replace('build_default_runtime(&config)', 'Arc::new(build_mapping(&config))')
    c = c.replace('build_default_runtime(&cfg)', 'Arc::new(build_mapping(&cfg))')
    c = c.replace('build_default_runtime(config)', 'Arc::new(build_mapping(config))')
    c = c.replace('build_default_runtime(cfg)', 'Arc::new(build_mapping(cfg))')

    # pipeline.runtime = Runtime { mapping: m, state: s } → pipeline.mapping = Arc::new(m); pipeline.state = s
    c = c.replace('let runtime = Runtime { mapping:', '// v3: runtime removed')
    c = c.replace('pipeline.runtime =', '// v3: use pipeline.mapping = Arc::new(build_mapping(...))')
    
    # pipeline.runtime = dev1_runtime → skip
    c = c.replace('let dev1_runtime = std::mem::replace(&mut pipeline.runtime,', 'let _dev1 = // v3: removed dev1_runtime')
    c = c.replace('let dev2_runtime = std::mem::replace(&mut pipeline.runtime,', 'let _dev2 = // v3: removed dev2_runtime')

    # remove .runtime from remaining patterns
    c = c.replace('pipeline.runtime.', 'pipeline.')
    c = c.replace('self.runtime.', 'self.')

    # RuntimeManager stuff in app_aware_test
    c = c.replace('manager.resolve(&pipeline.config,', 'manager.get_mapping(&pipeline.config,')
    
    # current_key
    c = c.replace('pipeline.current_key = (', '// v3: current_key')
    c = c.replace('&pipeline.current_key', '&(pipeline.current_device, pipeline.current_app.clone())')

    # Remove Runtime import
    c = c.replace('use anykey_engine::state::Runtime;', '')
    c = re.sub(r'\bRuntime,\s*', '', c)
    c = re.sub(r',\s*Runtime\b', '', c)

    # Remove Runtime from PipelineState init in test helpers
    c = c.replace('current_key: (1, String::new()),', 'current_domain: 1,\n        current_device: 1,\n        current_app: String::new(),')

    with open(fpath, 'w', encoding='utf-8') as f:
        f.write(c)
    print(f'  {fpath}')
print('done')
