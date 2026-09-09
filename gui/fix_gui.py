with open('main.py', 'r', encoding='utf-8') as f:
    lines = f.readlines()

# A. Replace tapDance write block → layers timing
for i, l in enumerate(lines):
    if "cfg[\"tapDance\"] =" in l and i+1 < len(lines) and "enabled" in lines[i+1]:
        lines[i] = '        # TD timing moved into layers\n'
        lines[i+1] = '        cfg.setdefault("layers", {})\n'
        lines[i+2] = '        cfg["layers"]["holdTerm"] = int(ht_val) if ht_val else 200\n'
        lines[i+3] = '        cfg["layers"]["doubleTapTerm"] = int(dt_val) if dt_val else 250\n'
        lines[i+4] = '        cfg["layers"]["doubleHoldTerm"] = int(dh_val) if dh_val else 200\n'
        j = i + 5
        while j < len(lines) and ("keys" in lines[j] or "enabled" in lines[j] or "tapDance" in lines[j]):
            lines[j] = ''
            j += 1
        break

# B. Fix except block (backup tapDance)
for i, l in enumerate(lines):
    if "cfg[\"tapDance\"] = master.get(\"tapDance\"," in l:
        lines[i] = '            cfg.setdefault("layers", {})\n'
        lines[i+1] = '            cfg["layers"]["holdTerm"] = 200\n'
        lines[i+2] = '            cfg["layers"]["doubleTapTerm"] = 250\n'
        lines[i+3] = '            cfg["layers"]["doubleHoldTerm"] = 200\n'
        j = i + 4
        while j < len(lines) and ("keys" in lines[j] or "enabled" in lines[j] or "tapDance" in lines[j]):
            lines[j] = ''
            j += 1
        break

# C. Remove blockHold
for i, l in enumerate(lines):
    if "blockHold" in l and "self._layer_block_vars" in l:
        lines[i] = l.split(', "blockHold"')[0] + '\n'

# D. Remove switchKeys / layers.enabled references
for i, l in enumerate(lines):
    if 'switch_keys_cfg' in l:
        lines[i] = ''
    if '"switchKeys": switch_keys,' in l or '"switchKeys": list(' in l:
        lines[i] = ''
    if 'switch_keys = existing_layers.get("switchKeys"' in l:
        lines[i] = '            _ = existing_layers.get("switchKeys", [])  # retired\n'
    if '"enabled": existing_layers.get("enabled"' in l or '"enabled": (g or {}).get("enabled"' in l:
        lines[i] = ''

# E. Clean double blank lines
cleaned = []
for i, l in enumerate(lines):
    if l.strip() == '' and i > 0 and cleaned and cleaned[-1].strip() == '':
        continue
    cleaned.append(l)

with open('main.py', 'w', encoding='utf-8') as f:
    f.writelines(cleaned)
print(f'done, {len(cleaned)} lines')
