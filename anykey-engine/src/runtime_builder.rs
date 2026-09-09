//! runtime_builder — builds Runtime { mapping, state } from config.
//! Also manages Runtime pool (RuntimeManager) for app-aware switching.

use crate::config::{ComboRow, Config, KeyEntry, LeaderSequence, Layers};
use crate::state::{DeviceMapping, DeviceContext, RuntimeDomain, LeaderDef, KeyDef, ComboIndex, DeviceState};
use crate::util::{norm_key, normalize_layer_name, regex_lazy, parse_u64_opt};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// ── Standalone builders ──

pub fn build_combo_index(combo_map: &[ComboRow]) -> ComboIndex {
    let mut map: HashMap<String, HashMap<String, HashMap<String, String>>> = HashMap::new();
    for row in combo_map {
        let k1 = norm_key(&row.key1);
        let k2 = norm_key(&row.key2);
        let output = row.output.trim().to_string();
        if k1.is_empty() || k2.is_empty() || output.is_empty() { continue; }
        let output = if output.len() == 1 { format!("{{{}}}", output) } else { output.clone() };
        let layer = normalize_layer_name(&row.layer);
        map.entry(layer.clone()).or_default()
            .entry(k1.clone()).or_default().insert(k2.clone(), output.clone());
        map.entry(layer.clone()).or_default()
            .entry(k2.clone()).or_default().insert(k1.clone(), output);
    }
    let mut key_set = HashSet::new();
    let mut partner_map: HashMap<String, HashSet<String>> = HashMap::new();
    let mut pairs: HashSet<(String, String)> = HashSet::new();
    for layer_map in map.values() {
        for (key, partners) in layer_map {
            key_set.insert(key.clone());
            for partner in partners.keys() {
                key_set.insert(partner.clone());
                let (a, b) = if key <= partner { (key.clone(), partner.clone()) } else { (partner.clone(), key.clone()) };
                pairs.insert((a, b));
            }
        }
    }
    for (a, b) in &pairs {
        partner_map.entry(a.clone()).or_default().insert(b.clone());
        partner_map.entry(b.clone()).or_default().insert(a.clone());
    }
    ComboIndex { map, key_set, partners: partner_map }
}

// ── per-device 条目级合并（全局 ∪ 设备覆盖，覆盖胜）──
// 覆盖桶只存被改条目；同 identity 时设备覆盖全局，否则继承全局。

/// combo 身份：(layer,key1,key2) 规范化（非空键小写、排序、'+' 连接，layer 前缀）。
/// 必须包含 layer！同一对按键可同时存在于 base / fn2 等多层
/// （如 a+s→{left}@base 与 a+s→{home}@fn2）。若只用 key1+key2，merge_combos 的
/// by 字典会发生键冲突，把另一层的同名组合覆盖掉（用户反馈：base a+s 被 fn2 a+s 覆盖失效）。
/// 与 GUI 的 _combo_ident（main.py）格式保持一致："layer:key1+key2"。
fn combo_identity(row: &ComboRow) -> String {
    let layer = normalize_layer_name(&row.layer);
    let mut ks: Vec<String> = [row.key1.clone(), row.key2.clone()]
        .into_iter().filter(|k| !k.trim().is_empty()).collect();
    for k in &mut ks { *k = norm_key(k); }
    ks.sort();
    format!("{}:{}", layer, ks.join("+"))
}

/// leader 身份：序列 keys（小写、',' 连接，顺序敏感）。
fn leader_identity(seq: &LeaderSequence) -> String {
    seq.keys.iter().map(|k| norm_key(k)).collect::<Vec<_>>().join(",")
}

pub fn merge_combos(global: &[ComboRow], ov: Option<&[ComboRow]>) -> Vec<ComboRow> {
    let mut by_id: HashMap<String, ComboRow> = HashMap::new();
    for r in global { by_id.insert(combo_identity(r), r.clone()); }
    if let Some(rows) = ov {
        for r in rows { by_id.insert(combo_identity(r), r.clone()); }
    }
    by_id.into_values().collect()
}

pub fn merge_leader(global: &[LeaderSequence], ov: Option<&[LeaderSequence]>) -> Vec<LeaderSequence> {
    let mut by_id: HashMap<String, LeaderSequence> = HashMap::new();
    for s in global { by_id.insert(leader_identity(s), s.clone()); }
    if let Some(seqs) = ov {
        for s in seqs { by_id.insert(leader_identity(s), s.clone()); }
    }
    by_id.into_values().collect()
}

/// 合并 layers：扁平格式，直接在 layer_maps 上层键级别覆盖。
pub fn merge_layers(global: &Layers, ov: Option<&HashMap<String, HashMap<String, KeyEntry>>>) -> Layers {
    let mut out = global.clone();
    if let Some(ov_map) = ov {
        for (layer_name, entries) in ov_map {
            let target = out.layer_maps.entry(layer_name.clone()).or_default();
            for (phys, entry) in entries {
                target.insert(phys.clone(), entry.clone());
            }
        }
    }
    out
}

pub fn build_tap_dance_map(layers: &Layers, combo_map: &[ComboRow]) -> HashMap<String, HashMap<String, KeyDef>> {
    let mut keys: HashMap<String, HashMap<String, KeyDef>> = HashMap::new();
    let base_layer = layers.base();
    let mut td_keys: HashMap<String, String> = HashMap::new();
    for (phys, entry) in base_layer {
        if entry.tap.is_empty() && entry.hold.is_empty()
            && entry.dt.is_empty() && entry.dh.is_empty() { continue; }
        td_keys.insert(norm_key(phys), phys.clone());
    }
    let switch_re = regex_lazy(r"^\{([fbt]n\d+)\}$");
    for (phys, entry) in base_layer {
        if let Some(caps) = switch_re.captures(&entry.hold) {
            let mut fn_name = caps[1].to_lowercase();
            if fn_name.starts_with("bn") || fn_name.starts_with("tn") { fn_name = format!("fn{}", &fn_name[2..]); }
            if !td_keys.contains_key(&norm_key(phys)) {
                td_keys.insert(norm_key(phys), phys.clone());
            }
        }
    }
    let fn_names = layers.fn_layers();
    let combo_index = build_combo_index(combo_map);
    let all_phys: HashSet<String> = {
        let mut s: HashSet<String> = HashSet::new();
        for phys in base_layer.keys() { s.insert(norm_key(phys)); }
        for nm in &fn_names {
            if let Some(km) = layers.get(nm) {
                for k in km.keys() { s.insert(norm_key(k)); }
            }
        }
        for phys in td_keys.keys() { s.insert(phys.clone()); }
        for k in &combo_index.key_set { s.insert(k.clone()); }
        s
    };
    for phys in &all_phys {
        let mut layer_map: HashMap<String, KeyDef> = HashMap::new();
        let base_entry = base_layer.get(phys).or_else(|| {
            base_layer.iter().find(|(k, _)| norm_key(k) == *phys).map(|(_, v)| v)
        });
        let tap = base_entry
            .and_then(|e| if e.tap.is_empty() { None } else { Some(e.tap.trim().to_string()) })
            .unwrap_or_else(|| format!("{{{}}}", phys));
        let (hold, double_tap, double_hold, ht, dtt, dht) = base_entry
            .map(|e| (e.hold.clone(), e.dt.clone(), e.dh.clone(),
                      parse_u64_opt(&e.ht), parse_u64_opt(&e.dtt), parse_u64_opt(&e.dht)))
            .unwrap_or_default();
        let hold = hold.trim().to_string();
        let double_tap = double_tap.trim().to_string();
        let double_hold = double_hold.trim().to_string();
        let mut base_def = KeyDef {
            tap, hold, double_tap, double_hold,
            hold_term: ht, dbl_tap_term: dtt, dbl_hold_term: dht
        };
        if base_def.tap.is_empty() && !base_def.hold.is_empty() { base_def.tap = format!("{{{}}}", phys); }
        layer_map.insert("base".to_string(), base_def);
        for nm in &fn_names {
            if let Some(km) = layers.get(nm) {
                let val = km.get(phys).or_else(|| km.iter().find(|(k, _)| norm_key(k) == *phys).map(|(_, v)| v));
                if let Some(val) = val {
                    let item = KeyDef {
                        tap: val.tap.trim().to_string(),
                        hold: val.hold.trim().to_string(),
                        double_tap: val.dt.trim().to_string(),
                        double_hold: val.dh.trim().to_string(),
                        hold_term: parse_u64_opt(&val.ht),
                        dbl_tap_term: parse_u64_opt(&val.dtt),
                        dbl_hold_term: parse_u64_opt(&val.dht)
                    };
                    layer_map.insert(nm.to_string(), item);
                }
            }
        }
        keys.insert(phys.clone(), layer_map);
    }
    keys
}

/// Build DeviceContext from global config (single-device mode).
/// Returns HashMap keyed by device ID — single-device → {1: DeviceContext}.
pub fn build_default_device_context(config: &Config) -> (HashMap<u32, DeviceContext>, HashMap<u32, RuntimeDomain>) {
    let tap_dance_map = build_tap_dance_map(&config.layers, &config.combo_map);
    let combo_index = build_combo_index(&config.combo_map);
    let leader = LeaderDef {
        sequences: config.leader.sequences.clone(),
        timeout_ms: config.leader.timeout_ms
    };
    let mut contexts = HashMap::new();
    let mut domains  = HashMap::new();
    domains.insert(1, RuntimeDomain::new(1));
    contexts.insert(1, DeviceContext {
        mapping: DeviceMapping {
            tap_dance: tap_dance_map,
            combo: combo_index,
            leader
        },
        domain_id: 1
    });
    (contexts, domains)
}

/// Build DeviceContext for each matched device.
/// Each device gets its own independent runtime state; mapping data merges global ∪ per-device override.
pub fn build_multi_device_contexts(
    config: &Config,
    device_ids: &[u32],
    subscribed: &HashSet<u32>,
) -> (HashMap<u32, DeviceContext>, HashMap<u32, RuntimeDomain>) {
    // 全局基础映射（预构建一次）
    let global_td = build_tap_dance_map(&config.layers, &config.combo_map);
    let global_combo = build_combo_index(&config.combo_map);
    let global_leader = LeaderDef {
        sequences: config.leader.sequences.clone(),
        timeout_ms: config.leader.timeout_ms
    };
    // 未订阅设备的空映射（纯透传）
    let empty_mapping = DeviceMapping {
        tap_dance: HashMap::new(),
        combo: ComboIndex { map: HashMap::new(), key_set: HashSet::new(), partners: HashMap::new() },
        leader: LeaderDef { sequences: vec![], timeout_ms: 1000 },
    };
    let mut contexts = HashMap::new();
    let mut domains  = HashMap::new();
    for &id in device_ids {
        let is_subscribed = subscribed.is_empty() || subscribed.contains(&id);
        let domain_id = config.subscribed_devices.iter()
            .find(|d| d.runtime_device_id == Some(id))
            .and_then(|d| d.domain_id)
            .unwrap_or(1);
        let domain_id = if domain_id == 0 { id } else { domain_id };

        domains.entry(domain_id).or_insert_with(|| RuntimeDomain::new(domain_id));

        let mapping = if !is_subscribed {
            empty_mapping.clone()
        } else {
            // 按设备 guid（无 guid 降级 VID:PID:type）查覆盖桶
            let sd = config.subscribed_devices.iter().find(|d| d.runtime_device_id == Some(id));
            let key = sd.and_then(|d| d.guid.clone())
                .or_else(|| sd.map(|d| format!("{}:{}:{}", d.vid, d.pid, d.kind)));
            let ov = key.as_ref().and_then(|k| config.devices.get(k));
            match ov {
                None => DeviceMapping {
                    tap_dance: global_td.clone(),
                    combo: global_combo.clone(),
                    leader: global_leader.clone()
                },
                Some(o) => {
                    let merged_layers = merge_layers(&config.layers, o.layers.as_ref());
                    let merged_combos = merge_combos(&config.combo_map, o.combo_map.as_deref());
                    let merged_leader = LeaderDef {
                        sequences: merge_leader(&config.leader.sequences, o.leader.as_deref()),
                        timeout_ms: config.leader.timeout_ms
                    };
                    DeviceMapping {
                        tap_dance: build_tap_dance_map(&merged_layers, &merged_combos),
                        combo: build_combo_index(&merged_combos),
                        leader: merged_leader
                    }
                }
            }
        };
        contexts.insert(id, DeviceContext { mapping, domain_id });
    }
    (contexts, domains)
}

// ── New Runtime builder functions ──

// build_default_runtime removed in v3 — use RuntimeManager.get_mapping

/// Compatibility wrapper for tests — builds Arc<DeviceMapping> from config.
pub fn build_default_runtime(config: &Config) -> std::sync::Arc<DeviceMapping> {
    let (ctxs, _) = build_default_device_context(config);
    std::sync::Arc::new(ctxs[&1].mapping.clone())
}

// build_device_runtime removed in v3 — use RuntimeManager.get_mapping

/// 应用感知四层合并链：
///   Layer 0: 全局 config.tapDance/comboMap/leader
///   Layer 1: config.appAware.apps[app]            ← 全局 app fallback
///   Layer 2: config.devices[guid]                  ← 设备覆盖（已含在 dev_mapping 内）
///   Layer 3: config.devices[guid].apps[app]        ← 设备+app 专属
pub fn apply_app_override(config: &Config, device_id: u32, app: &str, dev_mapping: &DeviceMapping) -> DeviceMapping {
    if app.is_empty() {
        return dev_mapping.clone();  // 无 app → 设备映射原样
    }

    // Layer 1: 全局 app 覆盖（大小写不敏感：进程名如 Revit.exe vs config key revit.exe）
    let app_ov = config.app_aware.apps.get(app)
        .or_else(|| config.app_aware.apps.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(app))
            .map(|(_, v)| v));
    let layers_after_app = if let Some(ov) = app_ov {
        merge_layers(&config.layers, ov.layers.as_ref())
    } else { config.layers.clone() };
    let combos_after_app = if let Some(ov) = app_ov {
        if let Some(ref rows) = ov.combo_map { merge_combos(&config.combo_map, Some(rows)) }
        else { config.combo_map.clone() }
    } else { config.combo_map.clone() };
    let leader_after_app = if let Some(ov) = app_ov {
        if let Some(ref seqs) = ov.leader { merge_leader(&config.leader.sequences, Some(seqs)) }
        else { config.leader.sequences.clone() }
    } else { config.leader.sequences.clone() };

    // Layer 2: 设备覆盖已在 dev_mapping 内（build_multi_device_contexts 已做）
    //   但我们对 layers/combos/leader 做的是从全局 base 重建——需合并设备层。
    //   捷径：用 dev_mapping 里的 tap_dance/combo 已经包含了设备覆盖，所以跳过 Layer 2。
    //   正确做法：从 config.devices 读原始覆盖，重新 merge。
    let sd = config.subscribed_devices.iter().find(|d| d.runtime_device_id == Some(device_id));
    let dev_key = sd.and_then(|d| d.guid.clone())
        .or_else(|| sd.map(|d| format!("{}:{}:{}", d.vid, d.pid, d.kind)));
    let dev_ov = dev_key.as_ref().and_then(|k| config.devices.get(k));

    let layers_after_dev = merge_layers(&layers_after_app, dev_ov.and_then(|o| o.layers.as_ref()));
    let combos_after_dev = merge_combos(&combos_after_app, dev_ov.and_then(|o| o.combo_map.as_deref()));
    let leader_after_dev = merge_leader(&leader_after_app, dev_ov.and_then(|o| o.leader.as_deref()));

    // Layer 3: 设备+app 专属覆盖
    let dev_app_ov = dev_ov.and_then(|o| o.apps.as_ref()).and_then(|apps| apps.get(app));
    let final_layers = merge_layers(&layers_after_dev, dev_app_ov.and_then(|o| o.layers.as_ref()));
    let final_combos = merge_combos(&combos_after_dev, dev_app_ov.and_then(|o| o.combo_map.as_deref()));
    let final_leader = merge_leader(&leader_after_dev, dev_app_ov.and_then(|o| o.leader.as_deref()));

    DeviceMapping {
        tap_dance: build_tap_dance_map(&final_layers, &final_combos),
        combo:     build_combo_index(&final_combos),
        leader:    LeaderDef { sequences: final_leader, timeout_ms: config.leader.timeout_ms }
    }
}

// ── RuntimeManager v3: mapping(device,app) + state(domain) ──

pub struct RuntimeManager {
    pub domains:  HashMap<u32, DeviceState>,
    pub mappings: HashMap<(u32, String), Arc<DeviceMapping>>,
}

impl RuntimeManager {
    pub fn new() -> Self {
        RuntimeManager { domains: HashMap::new(), mappings: HashMap::new() }
    }

    /// 为指定 app 预构建「订阅设备」的 mapping（app 切换时调用）。
    /// 设计要点（Runtime v3 + 应用感知）：
    ///   - 键鼠同 domain、高频交替输入 → 输入路径【只查不建】；
    ///   - 只在「构建 runtime 时」构建 map：app 切换后一般有几秒缓冲，
    ///     足够构建该 app 的 map，之后键鼠交替零成本换 Arc；
    ///   - 不一次构建全部 app（避免用不到的东西塞内存），按需按 app 构建；
    ///   - **订阅设备集由 perDevice（设备独立设置开关）决定**：
    ///       · perDevice=false（默认）→ 所有设备都经过全局映射，只做一份
    ///         「全局 × app」map 共享（内存一份）；
    ///       · perDevice=true → 只有被订阅设备（subscribed_devices 中 enabled 且
    ///         匹配 runtime_device_id）走各自 map，未订阅设备透传（不构建缓存）；
    ///         无订阅配置 → 全部未订阅，同样只做全局一份。
    pub fn build_app_mappings(&mut self, config: &Config, device_ids: &[u32], app: &str) {
        if !config.per_device {
            // 不启用独立设置：所有设备经过全局映射 → 一份「全局×app」map 共享
            self.build_global_mapping(config, device_ids, app);
            return;
        }
        // 启用独立设置：订阅设备集 = subscribed_devices 中 enabled 且匹配的设备
        let has_sub_cfg = config.subscribed_devices.iter()
            .any(|d| d.enabled && d.runtime_device_id.is_some());
        if !has_sub_cfg {
            // 无订阅配置 → 全部设备视为未订阅（透传），只有全局设置起作用
            self.build_global_mapping(config, device_ids, app);
            return;
        }
        let subscribed: HashSet<u32> = device_ids.iter().copied()
            .filter(|id| config.subscribed_devices.iter()
                .any(|d| d.enabled && d.runtime_device_id == Some(*id)))
            .collect();
        let (contexts, _) = build_multi_device_contexts(config, device_ids, &subscribed);
        for (device, dc) in &contexts {
            if !subscribed.contains(device) { continue; } // 只构建/缓存订阅设备
            let key = (*device, app.to_string());
            if self.mappings.contains_key(&key) { continue; }
            let mapping = apply_app_override(config, *device, app, &dc.mapping);
            self.mappings.insert(key, Arc::new(mapping));
        }
    }

    /// 构建一份「全局 × app」map，共享给所有设备（perDevice=false 或无订阅配置时）。
    fn build_global_mapping(&mut self, config: &Config, device_ids: &[u32], app: &str) {
        let (contexts, _) = build_multi_device_contexts(config, device_ids, &HashSet::new());
        if let Some(dc) = contexts.values().next() {
            let key = (0u32, app.to_string()); // device=0 代表全局
            if !self.mappings.contains_key(&key) {
                let mapping = apply_app_override(config, 0, app, &dc.mapping);
                let arc = Arc::new(mapping);
                self.mappings.insert(key, arc.clone());
                for d in device_ids {
                    self.mappings.insert((*d, app.to_string()), arc.clone());
                }
            }
        }
    }

    /// 取已构建的 (device, app) mapping（零构建，仅查表）。
    /// 返回 None 表示该组合未预构建（调用方应回退到设备默认 mapping）。
    pub fn lookup_mapping(&self, device: u32, app: &str) -> Option<Arc<DeviceMapping>> {
        self.mappings.get(&(device, app.to_string())).cloned()
    }

    /// Get or create Arc<DeviceMapping> for (device, app). Cached, zero-clone on reuse.
    pub fn get_mapping(&mut self, config: &Config, device: u32, app: &str) -> Arc<DeviceMapping> {
        let key = (device, app.to_string());
        self.mappings.entry(key.clone()).or_insert_with(|| {
            let (contexts, _) = build_multi_device_contexts(config, &[device], &[device].into_iter().collect());
            let dc = contexts.get(&device).expect("device context should exist");
            let mapping = apply_app_override(config, device, app, &dc.mapping);
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
}


