//! Matcher — device matching rules and alias lookup.
//!
//! Matcher resolves config rules against live descriptors.
//! Priority (highest confidence first):
//!   0. GUID + HardwareId     — precise; disambiguates shared-ContainerID composite devices
//!   1. GUID only             — backward-compat single-device containers
//!   2. VID/PID unique       — legacy devices with NO ContainerID (must be unique)
//!   3. RuntimeDeviceId lock — explicit pin for PS/locked legacy devices (no ContainerID)
//!   4. auto_reconnect       — stale-RID salvage for guid-less rules (VID/PID-no-serial)
//! All tiers filter by device type (keyboard/mouse) when the rule specifies a kind.
//! The volatile runtime_device_id is NEVER allowed to override a GUID-bearing
//! rule — a recycled instance number after re-plug/restart would bind the wrong device.

use crate::registry::DeviceDescriptor;
use std::collections::HashMap;

/// Check whether a descriptor's device type matches the rule's expected kind.
fn type_matches(desc: &DeviceDescriptor, kind: &str) -> bool {
    match kind {
        "keyboard" => desc.is_keyboard,
        "mouse"    => desc.is_mouse,
        _          => true,  // unknown/empty kind → match any
    }
}

// ── DeviceRule — from config.json ──

#[derive(Debug, Clone)]
pub struct DeviceRule {
    pub runtime_device_id: Option<u32>,
    pub guid:              Option<String>,
    pub vid:               u16,
    pub pid:               u16,
    pub kind:              String,  // "keyboard" | "mouse"
    pub hardware_id:       Option<String>, // exact HardwareId (most precise; composite-device disambiguation)
}

// ── Matcher ──

pub struct Matcher {
    pub rules:       HashMap<String, DeviceRule>,   // alias → rule
    pub id_by_alias: HashMap<String, u32>,            // alias → runtime_id
    pub alias_by_id: HashMap<u32, String>,            // runtime_id → alias
    /// Debug: when true, `resolve` records which tier matched / why not.
    pub debug: bool,
    pub debug_lines: Vec<String>,
}

impl Matcher {
    pub fn new() -> Self {
        Matcher {
            rules: HashMap::new(),
            id_by_alias: HashMap::new(),
            alias_by_id: HashMap::new(),
            debug: false,
            debug_lines: Vec::new(),
        }
    }

    /// Load rules from config. Build reverse lookup.
    pub fn load_rules(&mut self, rules: HashMap<String, DeviceRule>) {
        self.rules = rules;
        self.id_by_alias.clear();
        self.alias_by_id.clear();
    }

    /// Resolve a rule to a runtime device id.
    ///
    /// Stable identifiers win. A ContainerID (GUID) pins "which physical
    /// device"; the HardwareId disambiguates composite devices that share a
    /// ContainerID across keyboard+mouse sub-nodes. The volatile
    /// `runtime_device_id` (OS instance number) is used ONLY as a last
    /// resort for legacy / PS-style devices that report no ContainerID — it is
    /// never allowed to override a GUID-bearing rule, because a recycled
    /// instance number after a re-plug/restart would otherwise bind the
    /// wrong device.
    ///
    /// Tiers (highest confidence first):
    ///   0. GUID + HardwareId  — precise; disambiguates shared-ContainerID composite devices
    ///   1. GUID only           — backward-compat single-device containers
    ///   2. VID/PID unique     — legacy devices with no ContainerID (must be unique)
    ///   3. RuntimeDeviceId lock — explicit pin for PS/locked legacy devices (no ContainerID)
    ///   4. auto_reconnect     — stale RID salvage for guid-less rules (VID/PID-no-serial)
    pub fn resolve(&mut self, descriptors: &[DeviceDescriptor], rule: &DeviceRule) -> Option<u32> {
        if self.debug {
            self.debug_lines.push(format!(
                "[match] rule: guid={:?} vid={:04X} pid={:04X} kind='{}' rid={:?} hwid={:?}",
                rule.guid, rule.vid, rule.pid, rule.kind, rule.runtime_device_id, rule.hardware_id));
        }

        // Tier 0: GUID + HardwareId (most precise; disambiguates composite
        // devices that share a ContainerID across keyboard+mouse sub-devices).
        if let (Some(ref rg), Some(ref rh)) = (&rule.guid, &rule.hardware_id) {
            if self.debug {
                let candidates: Vec<String> = descriptors.iter()
                    .filter(|d| d.container_id.as_ref().map(|g| g.eq_ignore_ascii_case(rg)).unwrap_or(false))
                    .map(|d| format!("id={} hwid='{}' kbd={} mouse={} name='{}'",
                        d.runtime_device_id, d.hardware_id, d.is_keyboard, d.is_mouse, d.friendly_name))
                    .collect();
                if !candidates.is_empty() {
                    self.debug_lines.push(format!("  Tier0 candidates (GUID match): {}", candidates.join(", ")));
                } else {
                    self.debug_lines.push("  Tier0 no candidates: no descriptor has this GUID".into());
                }
            }
            for dev in descriptors {
                if dev.container_id.as_ref().map(|g| g.eq_ignore_ascii_case(rg)).unwrap_or(false)
                    && dev.hardware_id.eq_ignore_ascii_case(rh)
                    && type_matches(dev, &rule.kind)
                {
                    if self.debug {
                        self.debug_lines.push(format!(
                            "[match] Tier0 GUID+HWID HIT dev={} '{}' (hwid='{}')",
                            dev.runtime_device_id, dev.friendly_name, dev.hardware_id));
                    }
                    return Some(dev.runtime_device_id);
                }
            }
            if self.debug {
                self.debug_lines.push(format!(
                    "[match] Tier0 GUID+HWID NO HIT for guid={:?} hwid={:?}", rg, rh));
            }
        }

        // Tier 1: ContainerID GUID only (with type filter for composite devices).
        if let Some(ref rg) = rule.guid {
            for dev in descriptors {
                if dev.container_id.as_ref().map(|g| g.eq_ignore_ascii_case(rg)).unwrap_or(false)
                    && type_matches(dev, &rule.kind)
                {
                    if self.debug {
                        self.debug_lines.push(format!(
                            "[match] Tier1 GUID HIT  dev={} '{}' (container_id={:?})",
                            dev.runtime_device_id, dev.friendly_name, dev.container_id));
                    }
                    return Some(dev.runtime_device_id);
                }
            }
            if self.debug {
                self.debug_lines.push(format!("[match] Tier1 GUID NO HIT for {:?}", rg));
            }
        }

        // ── Below this point: legacy / PS-style devices that report NO
        // ContainerID. We must NOT consult the volatile `runtime_device_id`
        // for GUID-bearing rules — a recycled instance number would bind
        // the wrong device — so the RID / auto-reconnect tiers are
        // gated on `guid.is_none()`. ──
        if rule.guid.is_none() {
            // Tier 2: VID/PID — must be unique (with type filter).
            let matches: Vec<u32> = descriptors.iter()
                .filter(|d| rule.vid == d.vendor_id && rule.pid == d.product_id && type_matches(d, &rule.kind))
                .map(|d| d.runtime_device_id)
                .collect();
            if matches.len() == 1 {
                if self.debug {
                    self.debug_lines.push(format!("[match] Tier2 VID/PID HIT dev={}", matches[0]));
                }
                return Some(matches[0]);
            } else if !matches.is_empty() {
                if self.debug {
                    self.debug_lines.push(format!(
                        "[match] Tier2 VID/PID AMBIGUOUS ({} hits: {:?}) -> falling through",
                        matches.len(), matches));
                }
            }

            // Tier 3: RuntimeDeviceId lock (explicit pin for PS/locked
            // devices that report no ContainerID). Only valid if the pinned
            // id is still present and type-matches; otherwise we fall through.
            if let Some(locked) = rule.runtime_device_id {
                for dev in descriptors {
                    if dev.runtime_device_id == locked && type_matches(dev, &rule.kind) {
                        if self.debug {
                            self.debug_lines.push(format!(
                                "[match] Tier3 RID LOCK HIT dev={} '{}'",
                                dev.runtime_device_id, dev.friendly_name));
                        }
                        return Some(locked);
                    }
                }
                if self.debug {
                    self.debug_lines.push(format!(
                        "[match] Tier3 RID LOCK NO HIT (locked={:?} not present / type mismatch) -> falling through", locked));
                }
            }

            // Tier 4 (last resort): stale RID → VID/PID-no-serial auto-reconnect.
            // Triggered by the driver's device-change notification
            // (ANYKEY_FLAG_DEVICE_CHANGED) via a re-match: on hotplug the
            // engine re-scans and re-resolves; a guid-less rule whose pinned
            // runtime id went stale is salvaged here. Only attempted when the
            // rule actually HAD a pinned rid (otherwise we have no basis to
            // pick among duplicate VID/PID devices).
            if rule.runtime_device_id.is_some() {
                if let Some(new_id) = self.auto_reconnect(descriptors, rule) {
                    if self.debug {
                        self.debug_lines.push(format!(
                            "[match] Tier4 auto_reconnect -> dev={} (stale rid={:?})",
                            new_id, rule.runtime_device_id));
                    }
                    return Some(new_id);
                }
            }
        }

        if self.debug {
            self.debug_lines.push("[match] => NO MATCH (device not resolved)".into());
        }
        None
    }

    /// Auto-reconnect (legacy / PS-style fallback, used ONLY when a rule has NO GUID).
    ///
    /// For a rule whose pinned `runtime_device_id` went stale (device re-plugged →
    /// the OS assigns a new instance id), find a candidate with the same VID/PID
    /// and no serial number, and return its (fresh) runtime id.
    ///
    /// This is invoked as **Tier 4 (last resort)** from `resolve`, gated on
    /// `rule.guid.is_none()`. Devices that DO report a ContainerID are reconnected
    /// by the GUID+HWID tiers instead, so this is strictly the guid-less salvage
    /// path that the driver's device-change notification triggers via a re-match.
    pub fn auto_reconnect(
        &mut self,
        descriptors: &[DeviceDescriptor],
        rule: &DeviceRule,
    ) -> Option<u32> {
        let candidates: Vec<&DeviceDescriptor> = descriptors.iter()
            .filter(|d| d.vendor_id == rule.vid && d.product_id == rule.pid
                        && !d.has_serial && type_matches(d, &rule.kind))
            .collect();
        if candidates.is_empty() {
            if self.debug {
                self.debug_lines.push(format!(
                    "[match] Tier4 auto_reconnect: no VID/PID-no-serial candidate for {:04X}:{:04X}",
                    rule.vid, rule.pid));
            }
            return None;
        }
        // Prefer a candidate whose hardware_id matches the rule (closest to the
        // original physical node); otherwise fall back to the first.
        let chosen = candidates.iter()
            .find(|d| rule.hardware_id.as_deref() == Some(d.hardware_id.as_str()))
            .or_else(|| candidates.first());
        chosen.map(|d| d.runtime_device_id)
    }

    /// Lookup alias for debug logging.
    pub fn alias_for(&self, id: u32) -> &str {
        self.alias_by_id.get(&id).map(|s| s.as_str()).unwrap_or("unknown")
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::DeviceDescriptor;

    fn fake_descriptors() -> Vec<DeviceDescriptor> {
        vec![
            DeviceDescriptor {
                runtime_device_id: 1,
                container_id: Some("{11111111-1111-1111-1111-111111111111}".into()),
                hardware_id: "HID\\VID_046D&PID_C539&...".into(),
                friendly_name: "Logitech G Pro X".into(),
                vendor_id: 0x046D, product_id: 0xC539,
                alias: Some("主力键盘".into()),
                is_keyboard: true, is_mouse: false, has_serial: true, flags: 0,
            },
            DeviceDescriptor {
                runtime_device_id: 2,
                container_id: None,
                hardware_id: "HID\\VID_046D&PID_C539&...".into(),
                friendly_name: "Logitech G Pro X #2".into(),
                vendor_id: 0x046D, product_id: 0xC539,
                alias: Some("副键盘".into()),
                is_keyboard: true, is_mouse: false, has_serial: false, flags: 0,
            },
            DeviceDescriptor {
                runtime_device_id: 3,
                container_id: Some("{22222222-2222-2222-2222-222222222222}".into()),
                hardware_id: "HID\\VID_045E&PID_00CB&...".into(),
                friendly_name: "Microsoft Mouse".into(),
                vendor_id: 0x045E, product_id: 0x00CB,
                alias: None,
                is_keyboard: false, is_mouse: true, has_serial: true, flags: 0,
            },
        ]
    }

    #[test]
    fn test_guid_match_single() {
        let devs = fake_descriptors();
        let rule = DeviceRule {
            guid: Some("{11111111-1111-1111-1111-111111111111}".into()),
            vid: 0, pid: 0, kind: "keyboard".into(), runtime_device_id: None, hardware_id: None,
        };
        let mut m = Matcher::new();
        assert_eq!(m.resolve(&devs, &rule), Some(1));
    }

    #[test]
    fn test_guid_not_found() {
        let devs = fake_descriptors();
        let rule = DeviceRule {
            guid: Some("{DEADBEEF-DEAD-BEEF-DEAD-BEEFDEADBEEF}".into()),
            vid: 0, pid: 0, kind: "keyboard".into(), runtime_device_id: None, hardware_id: None,
        };
        let mut m = Matcher::new();
        assert_eq!(m.resolve(&devs, &rule), None);
    }

    #[test]
    fn test_vidpid_match_unique() {
        let devs = fake_descriptors();
        let rule = DeviceRule {
            guid: None,
            vid: 0x045E, pid: 0x00CB, kind: "mouse".into(), runtime_device_id: None, hardware_id: None,
        };
        let mut m = Matcher::new();
        assert_eq!(m.resolve(&devs, &rule), Some(3));
    }

    #[test]
    fn test_vidpid_duplicate_fails() {
        let devs = fake_descriptors();
        let rule = DeviceRule {
            guid: None,
            vid: 0x046D, pid: 0xC539, kind: "keyboard".into(), runtime_device_id: None, hardware_id: None,
        };
        let mut m = Matcher::new();
        assert_eq!(m.resolve(&devs, &rule), None, "duplicate VID/PID must return None");
    }

    #[test]
    fn test_runtime_id_lock() {
        let devs = fake_descriptors();
        let rule = DeviceRule {
            guid: None,
            vid: 0x046D, pid: 0xC539, kind: "keyboard".into(),
            runtime_device_id: Some(2),
            hardware_id: None,
        };
        let mut m = Matcher::new();
        assert_eq!(m.resolve(&devs, &rule), Some(2));
    }

    #[test]
    fn test_resolve_guidless_stale_uses_auto_reconnect() {
        // Legacy / PS-style device: rule has NO guid (no ContainerID),
        // but its pinned runtime_device_id (99) went stale after replug.
        // resolve must fall through GUID/VidPid tiers and reach Tier4
        // (auto_reconnect: same VID/PID + no serial -> fresh id).
        let devs = fake_descriptors();
        let mut m = Matcher::new();
        let rule = DeviceRule {
            guid: None,
            vid: 0x046D, pid: 0xC539, kind: "keyboard".into(),
            runtime_device_id: Some(99),  // stale — device 99 no longer present
            hardware_id: None,
        };
        assert_eq!(m.resolve(&devs, &rule), Some(2),
            "guid-less stale rule must auto-reconnect to the VID/PID-no-serial device (id=2)");
    }

    #[test]
    fn test_alias_lookup() {
        let devs = fake_descriptors();
        let mut m = Matcher::new();
        let mut rules = HashMap::new();
        rules.insert("主力键盘".into(), DeviceRule {
            guid: Some("{11111111-1111-1111-1111-111111111111}".into()),
            vid: 0, pid: 0, kind: "keyboard".into(), runtime_device_id: None, hardware_id: None,
        });
        m.load_rules(rules);
        let id = m.resolve(&devs, &m.rules.get("主力键盘").unwrap().clone()).unwrap();
        m.id_by_alias.insert("主力键盘".into(), id);
        m.alias_by_id.insert(id, "主力键盘".into());
        assert_eq!(m.alias_for(1), "主力键盘");
        assert_eq!(m.alias_for(99), "unknown");
    }

    #[test]
    fn test_auto_reconnect_finds_candidate() {
        let devs = fake_descriptors();
        let mut m = Matcher::new();
        let rule = DeviceRule {
            guid: None,
            vid: 0x046D, pid: 0xC539, kind: "keyboard".into(),
            runtime_device_id: Some(99),  // stale — device 99 no longer exists
            hardware_id: None,
        };
        let new_id = m.auto_reconnect(&devs, &rule);
        assert!(new_id.is_some(), "should find a same VID/PID no-serial candidate");
    }

    // ── DUMP: what does Rust ACTUALLY deserialize from the live config? ──
    #[test]
    fn dump_config_subscriptions() {
        use crate::config::Config;
        let cfg_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../anykey_config.json");
        let json = match std::fs::read_to_string(cfg_path) {
            Ok(s) => s,
            Err(_) => { eprintln!("dump: config not found"); return; }
        };
        let config: Config = match serde_json::from_str(&json) {
            Ok(c) => c,
            Err(e) => { eprintln!("dump: PARSE ERROR: {}", e); return; }
        };
        println!("dump: parsed OK; {} subscribed_devices", config.subscribed_devices.len());
        for (i, d) in config.subscribed_devices.iter().enumerate() {
            println!("  [{}] alias='{}' kind='{}' (Rust-seen) guid={:?} vid='{}' pid='{}' rid={:?} hwid={:?} domain_id={:?} enabled={}",
                i, d.alias, d.kind, d.guid, d.vid, d.pid, d.runtime_device_id, d.hardware_id, d.domain_id, d.enabled);
        }
    }

    // ── REGRESSION: matching MUST be deterministic across runs ──
    // Loads the REAL anykey_config.json and replays the engine's exact
    // subscription loop (main.rs) N times. The fix: registry.descriptors is
    // now a BTreeMap (stable iteration order by runtime_device_id) AND the
    // matcher checks the RuntimeDeviceId lock first (Tier 0). So `resolve()`
    // returns the SAME subscription set every run even when a container_id is
    // shared by multiple sub-devices — the "keyboard/mouse mapping lost in a
    // cycle" symptom must no longer reproduce.
    #[test]
    fn repro_load_config_several_times() {
        use crate::config::Config;
        use crate::registry::DeviceDescriptor;
        use std::collections::HashMap;

        let cfg_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../anykey_config.json");
        let json = match std::fs::read_to_string(cfg_path) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("repro: config not found at {}, skipping", cfg_path);
                return;
            }
        };
        let config: Config = serde_json::from_str(&json).expect("parse real config");

        // Mirror the config's container_ids:
        //   subs 0,1 share guid G1 -> devices 2,3
        //   subs 2,3 share guid G2 -> devices 4,5
        //   sub  4      guid G3 (046D/C232) -> device 7
        let g1 = "{8096e871-16c1-11f1-840f-00e2696d284a}";
        let g2 = "{cfd498c0-fa5d-11f0-83db-00e2696d284a}";
        let g3 = "{dea7c9b5-3558-11f1-8445-d4d853ecc42f}";
        let mk = |id: u32, cid: Option<&str>, vid: u16, pid: u16, kbd: bool, mouse: bool, name: &str| DeviceDescriptor {
            runtime_device_id: id,
            container_id: cid.map(|s| s.to_string()),
            hardware_id: String::new(),
            friendly_name: name.to_string(),
            vendor_id: vid, product_id: pid,
            alias: None,
            is_keyboard: kbd, is_mouse: mouse,
            has_serial: false, flags: 0,
        };
        // Model the composite-device reality: a container_id is shared by
        // multiple sub-devices, and the driver flags BOTH as keyboard AND
        // mouse (so the `kind` filter cannot disambiguate them). This is
        // exactly what makes `resolve()`'s first-match-wins non-deterministic.
        let devices = vec![
            mk(2, Some(g1), 0, 0, true,  true, "DEV-A(g1, kbd+mouse)"),
            mk(3, Some(g1), 0, 0, true,  true,  "DEV-B(g1, kbd+mouse)"), // same container_id as 2
            mk(4, Some(g2), 0, 0, true,  true, "DEV-C(g2, kbd+mouse)"),
            mk(5, Some(g2), 0, 0, true,  true,  "DEV-D(g2, kbd+mouse)"), // same container_id as 4
            mk(7, Some(g3), 0x046D, 0xC232, true, false, "Logi(g3)"),
        ];

        let runs = 40;
        let mut seen: HashMap<String, usize> = HashMap::new();
        // 先打印规则列表（只看一次）
        println!("\n--- Config rules ---");
        for (i, d) in config.subscribed_devices.iter().enumerate() {
            println!("  rule[{}] alias='{}' kind='{}' guid={:?} vid={} pid={} rid={:?} hwid={:?} enabled={}",
                i, d.alias, d.kind, d.guid, d.vid, d.pid, d.runtime_device_id, d.hardware_id, d.enabled);
        }
        println!("\n--- Synthetic descriptors ---");
        for d in &devices {
            println!("  dev id={} name='{}' guid={:?} vid={:04X} pid={:04X} kbd={} mouse={} hwid='{}'",
                d.runtime_device_id, d.friendly_name, d.container_id,
                d.vendor_id, d.product_id, d.is_keyboard, d.is_mouse, d.hardware_id);
        }

        for run in 0..runs {
            // Mirror the FIXED registry: a BTreeMap iterates in stable ascending
            // runtime_device_id order (no per-process random seed).
            let mut descriptors: Vec<DeviceDescriptor> = devices.clone();
            descriptors.sort_by_key(|d| d.runtime_device_id);
            let mut subscribed: std::collections::HashSet<u32> = std::collections::HashSet::new();
            let mut m = Matcher::new();
            m.debug = true;  // 打开 debug，收集 tier 信息
            for dev in &config.subscribed_devices {
                if !dev.enabled { continue; }
                let rule = DeviceRule {
                    runtime_device_id: dev.runtime_device_id,
                    guid: dev.guid.clone(),
                    vid: u16::from_str_radix(&dev.vid, 16).unwrap_or(0),
                    pid: u16::from_str_radix(&dev.pid, 16).unwrap_or(0),
                    kind: dev.kind.clone(),
                    hardware_id: dev.hardware_id.clone(),
                };
                if let Some(id) = m.resolve(&descriptors, &rule) {
                    subscribed.insert(id);
                    if run < 5 {
                        println!("run {}: alias='{}' -> MATCH dev={}", run, dev.alias, id);
                    }
                } else {
                    if run < 5 {
                        println!("run {}: alias='{}' -> NO MATCH", run, dev.alias);
                    }
                }
                if run < 5 {
                    for line in m.debug_lines.drain(..) { println!("    {}", line); }
                } else {
                    m.debug_lines.clear();
                }
            }
            let mut ids: Vec<u32> = subscribed.iter().copied().collect();
            ids.sort();
            let key = format!("{:?}", ids);
            *seen.entry(key.clone()).or_insert(0) += 1;
            if run < 16 {
                println!("repro run {:2}: subscribed_ids = {}", run, key);
            }
        }
        println!("repro: {} distinct subscribed-id sets across {} runs (expect 1)", seen.len(), runs);
        let mut ranked: Vec<(&String, &usize)> = seen.iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(a.1));
        for (k, c) in ranked {
            println!("        {}  x{}", k, c);
        }
        assert!(seen.len() == 1,
            "REGRESSION: matching MUST be DETERMINISTIC across runs \
             (stable BTreeMap iteration + Tier-0 RID lock). got {} distinct sets",
             seen.len());
    }
}
