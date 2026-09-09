# Runtime v3 step 6: fix tests

import os, re

for fpath in os.listdir('tests'):
    if not fpath.endswith('.rs'):
        continue
    fpath = f'tests/{fpath}'
    with open(fpath, 'r', encoding='utf-8') as f:
        c = f.read()

    # Replace self.runtime.state → self.state
    c = c.replace('.runtime.state.', '.state.')
    # Replace self.runtime.mapping → self.mapping
    c = c.replace('.runtime.mapping.', '.mapping.')

    # Fix pipeline.runtime = ... → pipeline.mapping = ... and pipeline.state = ...
    c = c.replace('pipeline.runtime =', '// v3: use pipeline.mapping + pipeline.state =')
    c = c.replace('p.runtime =', '// v3: use p.mapping + p.state =')

    # Fix direct field access to runtime
    c = c.replace('pipeline.runtime.state.keys', 'pipeline.state.keys')
    c = c.replace('pipeline.runtime.state.', 'pipeline.state.')
    c = c.replace('pipeline.runtime.mapping.', 'pipeline.mapping.')
    c = c.replace('let dev1_runtime = std::mem::replace(&mut pipeline.runtime,', 'let saved_state = pipeline.state.clone(); let saved_mapping = pipeline.mapping.clone(); // v3')
    c = c.replace('pipeline.runtime = dev1_runtime', 'pipeline.state = saved_state; pipeline.mapping = saved_mapping')

    # Remove unused Runtime import
    c = re.sub(r'use anykey_engine::state::\{.*Runtime,?\s*', 'use anykey_engine::state::{', c)
    c = c.replace('use anykey_engine::state::Runtime;', '// v3: no Runtime struct')
    c = c.replace(', Runtime}', '}')
    
    # Import Arc if needed
    c = c.replace('use anykey_engine::state::PipelineState;', 
                  'use std::sync::Arc;\nuse anykey_engine::state::PipelineState;')

    with open(fpath, 'w', encoding='utf-8') as f:
        f.write(c)
    print(f'  {fpath}')

print('\ntests done')
