# Runtime v3 step 3: New RuntimeManager in runtime_builder.rs

with open('src/runtime_builder.rs', 'r', encoding='utf-8') as f:
    c = f.read()

# Add Arc import
c = c.replace('use std::collections::{HashMap, HashSet};',
              'use std::collections::{HashMap, HashSet};\nuse std::sync::Arc;')

# Replace old RuntimeManager + PoolEntry with new one
old_mgr = """// ── RuntimeManager (pool with LRU eviction) ──

const MAX_POOL: usize = 32;

struct PoolEntry {
    runtime:   Runtime,
    last_used: u64,
}

pub struct RuntimeManager {
    pool:  HashMap<(u32, String), PoolEntry>,
    clock: u64,
}

impl RuntimeManager {
    pub fn new() -> Self {
        RuntimeManager { pool: HashMap::new(), clock: 0 }
    }

    pub fn resolve(&mut self, config: &Config, device: u32, app: &str,
                   active_key: &(u32, String)) -> &mut Runtime
    {
        let key = (device, app.to_string());

        // LRU eviction
        if self.pool.len() >= MAX_POOL && !self.pool.contains_key(&key) {
            if let Some(oldest) = self.pool.iter()
                .filter(|(k, _)| *k != active_key && k.1 != "_default")
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            {
                self.pool.remove(&oldest);
            }
        }

        let entry = self.pool.entry(key.clone()).or_insert_with(|| PoolEntry {
            runtime: if config.app_aware.apps.contains_key(app) {
                build_device_runtime(config, device, app)
            } else {
                build_default_runtime(config)
            },
            last_used: 0
        });
        self.clock += 1;
        entry.last_used = self.clock;
        &mut entry.runtime
    }
}"""

new_mgr = """// ── RuntimeManager v3: mapping(device,app) + state(domain) ──

pub struct RuntimeManager {
    pub domains:  HashMap<u32, DeviceState>,
    pub mappings: HashMap<(u32, String), Arc<DeviceMapping>>,
    clock:        u64,
}

impl RuntimeManager {
    pub fn new() -> Self {
        RuntimeManager { domains: HashMap::new(), mappings: HashMap::new(), clock: 0 }
    }

    /// Get or create Arc<DeviceMapping> for (device, app). Cached, zero-clone on reuse.
    pub fn get_mapping(&mut self, config: &Config, device: u32, app: &str) -> Arc<DeviceMapping> {
        let key = (device, app.to_string());
        self.mappings.entry(key.clone()).or_insert_with(|| {
            let mapping = if config.app_aware.apps.contains_key(app) {
                let (contexts, _) = build_multi_device_contexts(config, &[device], &[device].into_iter().collect());
                let dc = contexts.get(&device).expect("device context should exist");
                apply_app_override(config, app, &dc.mapping)
            } else {
                let (contexts, _) = build_multi_device_contexts(config, &[device], &[device].into_iter().collect());
                contexts.get(&device).expect("device context should exist").mapping.clone()
            };
            Arc::new(mapping)
        }).clone()
    }

    /// Load domain state. Creates new if domain not yet known.
    pub fn load_domain_state(&mut self, domain_id: u32) -> DeviceState {
        self.domains.entry(domain_id).or_insert_with(DeviceState::default).clone()
    }

    /// Save domain state back to manager.
    pub fn save_domain_state(&mut self, domain_id: u32, state: DeviceState) {
        self.domains.insert(domain_id, state);
    }
}"""
c = c.replace(old_mgr, new_mgr)

# Remove old build_default_runtime (no longer needed)
old_br = """/// 构建默认单设备 Runtime（映射 = 设备默认，状态 = 空白）。
pub fn build_default_runtime(config: &Config) -> Runtime {
    let mapping = DeviceMapping {
        tap_dance: build_tap_dance_map(&config.layers, &config.combo_map),
        combo:     build_combo_index(&config.combo_map),
        leader:    LeaderDef {
            sequences:  config.leader.sequences.clone(),
            timeout_ms: config.leader.timeout_ms,
        },
    };
    Runtime { mapping, state: DeviceState::default() }
}
"""
c = c.replace(old_br, '// build_default_runtime removed in v3 — use RuntimeManager.get_mapping\n')

# Remove build_device_runtime (merged into get_mapping)
old_dr = """/// 按 (设备, 应用) 构建 Runtime。device_id 用于查 subscribed_devices 的覆盖桶，
/// app 参数预留（当前传 ""），未来用于 appAware 配置段覆盖。
pub fn build_device_runtime(config: &Config, device_id: u32, app: &str) -> Runtime {
    let ids = [device_id];
    let subscribed: HashSet<u32> = [device_id].into_iter().collect();
    let (contexts, domains) = build_multi_device_contexts(config, &ids, &subscribed);

    let dc = contexts.get(&device_id)
        .expect("build_multi_device_contexts should include device_id");
    let domain = domains.get(&dc.domain_id)
        .expect("domain should exist");

    // Apply app-aware override on top of device-level mapping
    let mapping = apply_app_override(config, app, &dc.mapping);

    Runtime { mapping, state: domain.state.clone() }
}
"""
c = c.replace(old_dr, '// build_device_runtime removed in v3 — use RuntimeManager.get_mapping\n')

# Remove Runtime import from test module
c = c.replace('use super::*;', '// v3: no Runtime struct\nuse super::*;')

# Update test module — remove references to Runtime struct
c = c.replace('use crate::config::{ComboRow, Config, DeviceInfo, DeviceOverride, KeyEntry, LeaderSequence};',
              'use crate::config::{ComboRow, Config, DeviceInfo, DeviceOverride, KeyEntry, LeaderSequence};\nuse crate::state::DeviceMapping;')

with open('src/runtime_builder.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('runtime_builder.rs updated')
