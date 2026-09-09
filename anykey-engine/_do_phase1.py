import re
# Restore base + apply Phase 1
p = open('src/pipeline.rs').read()
# Add helper
old = 'pub(crate) fn dev_runtime(&self) -> &crate::state::DeviceRuntime {\n        &self.contexts.get(&1).unwrap().runtime\n    }'
p = p.replace(old, old + '\n    pub(crate) fn dev_runtime_mut(&mut self) -> &mut crate::state::DeviceRuntime {\n        &mut self.contexts.get_mut(&1).unwrap().runtime\n    }')
open('src/pipeline.rs','w').write(p)

# Apply sed replace
import subprocess
subprocess.run(['sed','-i',
    's/self\.keys\.key_states/self.dev_runtime_mut().keys.key_states/g',
    's/self\.keys\.pending_timers/self.dev_runtime_mut().keys.pending_timers/g',
    'src/pipeline.rs'], check=True)

p = open('src/pipeline.rs').read()
lines = p.split('\n')

# Detect fns with dev_runtime_mut or dev_mapping
fn_starts = {}
for i,l in enumerate(lines):
    m = re.match(r'    (?:pub(?:\(crate\))? )?fn (\w+)', l)
    if m: fn_starts[m.group(1)] = i

fns = set()
for i,l in enumerate(lines):
    if 'self.dev_runtime_mut(' in l or 'self.dev_mapping(' in l:
        best_fn, best_line = '', -1
        for fn, n in fn_starts.items():
            if n < i and n > best_line: best_line, best_fn = n, fn
        if best_fn and best_fn not in ('dev_mapping','dev_runtime','dev_runtime_mut','sync_runtime_to_ctx'):
            fns.add(best_fn)

# Filter out &self fns
fns_final = set()
for fn in fns:
    ln = fn_starts[fn]
    sig = lines[ln]
    if '&self' in sig and '&mut' not in sig:
        continue  # skip &self fns
    fns_final.add(fn)

print(f'Fns to destructure: {fns_final}')

# Global replace
p = p.replace('self.dev_runtime_mut()', 'dev.runtime')
p = p.replace('self.dev_mapping()', 'dev.mapping')
lines2 = p.split('\n')

# Destructure
out = []; i = 0
while i < len(lines2):
    l = lines2[i]
    m = re.match(r'    (?:pub(?:\(crate\))? )?fn (\w+)', l)
    if m and m.group(1) in fns_final:
        out.append(l); j = i + 1; done = False
        # Check if { is on this line
        if l.rstrip().endswith('{'):
            out.append('        let dev = self.contexts.get_mut(&1).unwrap();')
            i = j; continue
        while j < len(lines2):
            cur = lines2[j]
            if not done and cur.rstrip() == '{':
                indent = cur[:len(cur)-len(cur.lstrip())] + '    '
                out.append(cur); out.append(indent + 'let dev = self.contexts.get_mut(&1).unwrap();')
                done = True; j += 1; break
            # Also handle if { is at end of a non-sig line (params continuation)
            if not done and '{' in cur and 'fn ' not in cur:
                indent = '        '
                out.append(cur); out.append(indent + 'let dev = self.contexts.get_mut(&1).unwrap();')
                done = True; j += 1; break
            out.append(cur); j += 1
        i = j
    else:
        out.append(l); i += 1

open('src/pipeline.rs','w').write('\n'.join(out))
