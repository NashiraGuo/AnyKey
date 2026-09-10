// ═══════════════════════════════════════════════════════════════
// main.rs — AnyKey Engine (filter-driver backend)
// Output: AnyKey FLT / SendInput / ShellExecute
// ═══════════════════════════════════════════════════════════════
#![windows_subsystem = "windows"]

use anykey_engine::state::{PipelineState, EmitEvent, TimerKind, DeviceMapping};
use anykey_engine::config::Config;
use anykey_engine::emit::{key_name_to_scancode, send_key_down_sendinput, send_key_up_sendinput};
use anykey_engine::app_sensor::AppSensor;
use anykey_engine::runtime_builder::RuntimeManager;
use anykey_engine::util::is_layer_key;
use anykey_engine::registry::{Registry, DeviceDescriptor};
use anykey_engine::matcher::{Matcher, DeviceRule};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::collections::HashSet;
use std::collections::HashMap;

#[cfg(feature = "filter-driver")]
use anykey_engine::filter_driver::{
    FilterDriver, ANYKEY_FLAG_DEVICE_CHANGED, AnyKeyOutputEvent, AnyKeyMouseOutputEvent,
    ANYKEY_KEY_BREAK, MouseEventTranslator,
};

// ── Global log file ──
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

macro_rules! log {
    ($($arg:tt)*) => {{
        if let Ok(mut guard) = LOG_FILE.lock() {
            if let Some(ref mut f) = *guard {
                let _ = writeln!(f, $($arg)*);
                let _ = f.flush();
            }
        }
    }};
}

/// 跨进程对齐时间戳：真实墙钟（UTC+8，µs 精度）+ QPC 原始计数。
/// 与 tools/keylatency 探针输出同一时间基，可逐事件计算「发出 → 系统接收」延迟。
/// 注意：与管道 DBG 的 [wall.tick%1000.qpc%1000]（引擎相对时基）不同，此为绝对墙钟。
fn ts_tag() -> String {
    use std::time::SystemTime;
    let us = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    // 当天秒内部分 → HH:MM:SS.mmm.uuu（UTC+8）
    let day_us = us % 86_400_000_000;           // UTC 当天微秒
    let cst_us = (day_us + 8 * 3_600_000_000) % 86_400_000_000;
    let h = cst_us / 3_600_000_000;
    let m = (cst_us / 60_000_000) % 60;
    let s = (cst_us / 1_000_000) % 60;
    let frac = cst_us % 1_000_000;
    // QPC 原始计数（系统级单调时钟，跨进程可比）
    let mut qpc: i64 = 0;
    unsafe {
        let _ = windows_sys::Win32::System::Performance::QueryPerformanceCounter(&mut qpc);
    }
    format!("[{:02}:{:02}:{:02}.{:03}.{:03} QPC={}]", h, m, s, frac / 1000, frac % 1000, qpc)
}

// ── MessageBox helper ──
#[cfg(windows)]
fn msgbox(title: &str, msg: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    let wide_title: Vec<u16> = OsStr::new(title).encode_wide().chain(Some(0)).collect();
    let wide_msg: Vec<u16> = OsStr::new(msg).encode_wide().chain(Some(0)).collect();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
            std::ptr::null_mut(), wide_msg.as_ptr(), wide_title.as_ptr(), 0x30);
    }
}
#[cfg(not(windows))]
fn msgbox(_title: &str, _msg: &str) {}

// ── scancode → key name (shared by all backends) ──
#[cfg(feature = "filter-driver")]
#[cfg(feature = "filter-driver")]
const KEY_E0: u16 = 0x02;
#[cfg(feature = "filter-driver")]
const KEY_E1: u16 = 0x04;

fn input_name(code: u16, state: u16) -> String {
    let e0 = (state & KEY_E0) != 0;
    let e1 = (state & KEY_E1) != 0;

    // Media keys (only when E0 prefix)
    if e0 {
        match code {
            0x10 => return "media_prev".into(),
            0x19 => return "media_next".into(),
            0x20 => return "volume_mute".into(),
            0x22 => return "media_play_pause".into(),
            0x24 => return "media_stop".into(),
            0x2E => return "volume_down".into(),
            0x30 => return "volume_up".into(),
            0x32 => return "browser_home".into(),
            0x5D => return "apps".into(),
            0x5E => return "browser_search".into(),
            0x6A => return "browser_back".into(),
            0x69 => return "browser_forward".into(),
            0x6B => return "browser_refresh".into(),
            0x6C => return "browser_favorites".into(),
            _ => {}
        }
    }

    if e1 { return "pause".into(); }

    if e0 {
        return match code {
            0x1C => "numpadenter", 0x35 => "numpaddiv",
            0x37 => "printscreen",  0x38 => "ralt",
            0x1D => "rctrl",        0x47 => "home",
            0x48 => "up",           0x49 => "pgup",
            0x4B => "left",         0x4D => "right",
            0x4F => "end",          0x50 => "down",
            0x51 => "pgdn",         0x52 => "insert",
            0x53 => "delete",       0x5B => "lwin",
            0x5C => "rwin",         0x5D => "apps",
            _ => return format!("e0-{:02x}", code),
        }.into();
    }

    match code {
        0x01 => "escape",    0x0E => "backspace", 0x0F => "tab",
        0x1C => "enter",     0x1D => "lctrl",
        0x2A => "lshift",    0x36 => "rshift",
        0x38 => "lalt",      0x39 => "space",
        0x3A => "capslock",  0x3B => "f1",  0x3C => "f2",
        0x3D => "f3",        0x3E => "f4",  0x3F => "f5",
        0x40 => "f6",        0x41 => "f7",  0x42 => "f8",
        0x43 => "f9",        0x44 => "f10", 0x45 => "numlock",
        0x46 => "scrolllock", 0x37 => "numpadmul",
        0xFF => "keyboard_reset",
        0x47 => "numpad7",   0x48 => "numpad8",  0x49 => "numpad9",
        0x4A => "numpadsub", 0x4B => "numpad4",  0x4C => "numpad5",
        0x4D => "numpad6",   0x4E => "numpadadd",0x4F => "numpad1",
        0x50 => "numpad2",   0x51 => "numpad3",  0x52 => "numpad0",
        0x53 => "numpaddot", 0x57 => "f11",      0x58 => "f12",
        0x5D => "apps",
        0x02 => "1",0x03=>"2",0x04=>"3",0x05=>"4",0x06=>"5",0x07=>"6",0x08=>"7",0x09=>"8",
        0x0A=>"9",0x0B=>"0",0x0C=>"-",0x0D=>"=",
        0x10=>"q",0x11=>"w",0x12=>"e",0x13=>"r",0x14=>"t",0x15=>"y",0x16=>"u",0x17=>"i",
        0x18=>"o",0x19=>"p",0x1A=>"[",0x1B=>"]",
        0x1E=>"a",0x1F=>"s",0x20=>"d",0x21=>"f",0x22=>"g",0x23=>"h",0x24=>"j",
        0x25=>"k",0x26=>"l",0x27=>";",0x28=>"'",0x29=>"`",
        0x2B=>"\\",0x2C=>"z",0x2D=>"x",0x2E=>"c",0x2F=>"v",0x30=>"b",0x31=>"n",0x32=>"m",
        0x33=>",",0x34=>".",0x35=>"/",
        _ => return format!("?{:02x}", code),
    }.into()
}

fn needs_e0(kn: &str) -> bool {
    let k = kn.to_lowercase();
    matches!(k.as_str(),
        "home"|"end"|"up"|"down"|"left"|"right"|
        "pgup"|"pgdn"|"pageup"|"pagedown"|"insert"|"delete"|
        "rctrl"|"ralt"|"rwin"|"lwin"|"apps"|
        "printscreen"|"numpadenter"|"numpaddiv"|
        "media_next"|"media_prev"|"media_stop"|"media_play_pause"|
        "volume_mute"|"volume_down"|"volume_up"|
        "browser_home"|"browser_search"|
        "browser_back"|"browser_forward"|
        "browser_refresh"|"browser_favorites"
    )
}

/// Convert mouse event name to button flags for output injection.
/// Returns (button_flags, is_wheel) where is_wheel means self-contained tap.
// main — Filter Driver backend (feature = "filter-driver")
// ═══════════════════════════════════════════════════════════════

#[cfg(feature = "filter-driver")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        msgbox("AnyKey Engine [FLT]", "Usage: anykey-engine.exe <config.json> [--debug]");
        return;
    }
    let config_path = &args[1];
    let debug_enabled = args.get(2).map_or(false, |a| a == "--debug");

    let log_path = std::path::Path::new(config_path)
        .parent().unwrap_or_else(|| std::path::Path::new("."))
        .join("anykey_engine.log")
        .to_string_lossy()
        .to_string();

    if debug_enabled {
        let mut guard = LOG_FILE.lock().unwrap();
        *guard = match std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&log_path) {
            Ok(f) => Some(f),
            Err(e) => { msgbox("AnyKey Engine [FLT]", &format!("Cannot create log:\n{}", e)); return; }
        };
    }

    log!("AnyKey Engine v0.1.0 [FLT]");
    log!("Config: {}", config_path);

    let json = match std::fs::read_to_string(config_path) {
        Ok(s) => { log!("Config loaded: {} bytes", s.len()); s }
        Err(e) => { log!("Cannot read config: {}", e); msgbox("AnyKey Engine [FLT]", &format!("Cannot read config:\n{}", e)); return; }
    };
    let config: Config = match serde_json::from_str::<Config>(&json) {
        Ok(c) => {
            log!("Parsed: {} combos, {} layer keys ({} layers)",
                c.combo_map.len(), c.layers.layer_maps.values().map(|m| m.len()).sum::<usize>(),
                c.layers.layer_maps.len());
            c
        }
        Err(e) => { log!("Invalid config: {}", e); msgbox("AnyKey Engine [FLT]", &format!("Invalid config:\n{}", e)); return; }
    };

    let mut pipeline = PipelineState::new(config);
    pipeline.debug_enabled = debug_enabled;

    // ── 调试：启动即打印初始 mapping（含 combo map 结构，排查「同键多层 combo 覆盖」）──
    if debug_enabled {
        dump_mapping(&pipeline.mapping, 0, "");
    }

    // ── 多设备集成：registry + matcher → per-device contexts ──
    let mut registry = Registry::new();

    let mut fd = match FilterDriver::open() {
        Ok(f) => { log!("Filter driver opened"); f }
        Err(e) => {
            log!("Filter driver FAILED: {}", e);
            msgbox("AnyKey Engine [FLT]", &format!("Failed to open filter driver:\n{}\n\nEnsure the driver is installed (run Install_AnyKey_Filter.bat).", e));
            return;
        }
    };

    match fd.register_event() {
        Ok(_) => log!("Event registered"),
        Err(e) => { log!("register_event FAILED: {}", e); msgbox("AnyKey Engine [FLT]", &format!("Event registration failed:\n{}", e)); return; }
    }

    // Drain stale events that accumulated before engine started
    match fd.poll_input() {
        Ok(v) => log!("Stale events drained: {}", v.len()),
        Err(e) => log!("Drain warning: {}", e),
    }

    // ── 多设备匹配：registry + matcher → per-device contexts ──
    // v0.4: registry.init() comes BEFORE per-device intercept, so we know
    // which devices exist before subscribing. Default = all passthrough (safe).
    let mut matcher = Matcher::new();
    matcher.debug = debug_enabled;
    let mut subscribed_ids: HashSet<u32> = HashSet::new();
    let descriptors: Vec<_> = if let Err(e) = registry.init(&fd) {
        log!("Registry scan: {} — using single-device fallback", e);
        Vec::new()
    } else {
        let descriptors: Vec<_> = registry.descriptors.values().cloned().collect();
        let ids: Vec<u32> = descriptors.iter().map(|d| d.runtime_device_id).collect();
        // ── 调试：把枚举到的全部设备名称/guid/vid/pid/类型打印出来 ──
        if debug_enabled {
            log!("  [dev-scan] {} device(s) enumerated from driver:", descriptors.len());
            for d in &descriptors {
                log!("    [dev] id={} name='{}' guid={:?} vid={:04X} pid={:04X} kbd={} mouse={} hwid='{}'",
                     d.runtime_device_id, d.friendly_name, d.container_id,
                     d.vendor_id, d.product_id, d.is_keyboard, d.is_mouse,
                     d.hardware_id);
            }
        }
        // 标记被用户显式订阅的设备（未订阅设备使用空映射=纯透传）
        if !pipeline.config.subscribed_devices.is_empty() {
            for dev in &pipeline.config.subscribed_devices {
                if !dev.enabled {
                    if debug_enabled {
                        log!("  [sub] SKIP disabled: alias='{}' guid={:?} rid={:?} kind='{}'",
                             dev.alias, dev.guid, dev.runtime_device_id, dev.kind);
                    }
                    continue;
                }
                let rule = DeviceRule {
                    runtime_device_id: dev.runtime_device_id,
                    guid: dev.guid.clone(),
                    vid: u16::from_str_radix(&dev.vid, 16).unwrap_or(0),
                    pid: u16::from_str_radix(&dev.pid, 16).unwrap_or(0),
                    kind: dev.kind.clone(),
                    hardware_id: dev.hardware_id.clone(),
                };
                if let Some(id) = matcher.resolve(&descriptors, &rule) {
                    let name = descriptors.iter()
                        .find(|d| d.runtime_device_id == id)
                        .map(|d| d.friendly_name.as_str())
                        .unwrap_or("?");
                    log!("  [sub] alias='{}' guid={:?} vid={:04X} pid={:04X} kind='{}' rid={:?} => MATCH device_id={} name='{}'",
                         dev.alias, dev.guid, rule.vid, rule.pid, dev.kind, dev.runtime_device_id, id, name);
                    subscribed_ids.insert(id);
                } else {
                    log!("  [sub] alias='{}' guid={:?} vid={:04X} pid={:04X} kind='{}' rid={:?} => NO MATCH (device not resolved)",
                         dev.alias, dev.guid, rule.vid, rule.pid, dev.kind, dev.runtime_device_id);
                }
                // 把 matcher 内部的 tier 调试信息落盘
                for line in matcher.debug_lines.drain(..) { log!("    {}", line); }
            }
        }
        if !ids.is_empty() {
            log!("Multi-device: {} device(s), {} subscribed, pipeline initialized",
                 ids.len(), subscribed_ids.len());
            if debug_enabled {
                for id in &ids {
                    let sub = subscribed_ids.contains(id);
                    let sd = pipeline.config.subscribed_devices.iter().find(|d| d.runtime_device_id == Some(*id));
                    let key = sd.and_then(|d| d.guid.clone())
                        .or_else(|| sd.map(|d| format!("{}:{}:{}", d.vid, d.pid, d.kind)));
                    let ov = key.as_ref().and_then(|k| pipeline.config.devices.get(k));
                    let name = descriptors.iter().find(|d| d.runtime_device_id == *id)
                        .map(|d| d.friendly_name.as_str()).unwrap_or("?");
                    log!("    [ctx] device_id={} name='{}' subscribed={} override_key={:?} has_override={} has_slot={}",
                         id, name, sub, key, ov.is_some(), true);
                }
            }
        }
        descriptors
    };

    // ── v0.4: per-device intercept — only subscribed devices are intercepted ──
    // Mouse merged-packet translator: per-device pressed-mask state.
    let mut mouse_translator = MouseEventTranslator::new();
    if !descriptors.is_empty() {
        // Ensure ALL devices start in passthrough (safe default)
        if let Err(e) = fd.set_device_intercept(0, false) {
            log!("set_device_intercept(all, false) warning: {}", e);
        }
        // Enable interception only for subscribed devices
        for &id in &subscribed_ids {
            match fd.set_device_intercept(id, true) {
                Ok(_) => log!("[intercept] device {} → INTERCEPT ON", id),
                Err(e) => log!("[intercept] device {} FAILED: {}", id, e),
            }
        }
        log!("Per-device intercept: {} subscribed, {} total device(s)",
             subscribed_ids.len(), descriptors.len());
    }
    // New session: translator state starts clean (no stale pressed masks).
    mouse_translator.reset();

    // ── 初始化活跃设备默认值：用 registry 中第一个键盘/鼠标作为 fallback ──
    let default_kbd: u32 = descriptors.iter()
        .find(|d| d.is_keyboard).map(|d| d.runtime_device_id).unwrap_or(0);
    let default_ms:  u32 = descriptors.iter()
        .find(|d| d.is_mouse).map(|d| d.runtime_device_id).unwrap_or(0);
    // 全部枚举设备 id（prebuild 用）
    let all_device_ids: Vec<u32> = descriptors.iter().map(|d| d.runtime_device_id).collect();
    log!("Default devices: keyboard={}, mouse={}", default_kbd, default_ms);

    // ── Heartbeat watchdog thread ──
    // Opens a separate handle to \\.\AnyKeyFlt and sends heartbeat IOCTL
    // every 5s. This lives in its own thread so an input-thread hang doesn't
    // kill the heartbeat. If the driver misses 30s of heartbeats, it performs
    // emergency shutdown (flush queues, release modifiers, disable intercept).
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    let heartbeat_running = Arc::new(AtomicBool::new(true));
    {
        let running = heartbeat_running.clone();
        std::thread::spawn(move || {
            // Open separate handle for heartbeat — isolated from input thread
            let hb_fd = match FilterDriver::open() {
                Ok(f) => f,
                Err(e) => { log!("  HEARTBEAT: {}", e); return; }
            };

            while running.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_secs(5));
                if !running.load(Ordering::Relaxed) { break; }

                match hb_fd.heartbeat() {
                    Ok(_) => { /* healthy */ }
                    Err(e) => log!("  HEARTBEAT: {}", e),
                }
            }
        });
    }

    let dev_count = fd.get_device_count().unwrap_or(0);
    log!("Devices: {}", dev_count);

    //
    // TODO: 设备管理（active_device）当前由 I/O 层在 main loop 中维护。
    //
    // active_device 的作用：
    //   - 记录最近一次按键事件来源的设备 ID（DeviceId）。
    //   - 所有输出注入（send_output）均使用此 ID，保证「从哪个设备拦截的
    //     按键，就注回哪个设备」。
    //   - 单设备场景下天然正确；多设备场景下，任何按键都会更新此值，
    //     注入总是发往最后一个活跃设备。
    //
    // 未来方向：
    //   当引擎支持「每设备独立映射」时，设备路由应由 Pipeline 负责——
    //   即 Context.device_id 贯穿整个管道，EmitEvent 携带 device_id，
    //   由 I/O 层（drain_emit_log_flt）直接取用，不再于此维护 active_device。
    //   届时需调整：① EmitEvent 枚举变体加 device_id 字段；
    //   ② key_down 传参；③ 移除 active_device。
    //

    // ── Drain: 键盘事件 → recent_keyboard, 鼠标事件 → recent_mouse ──
    // 输出目标 = EmitEvent 自带的触发设备 (ev_dev，emit 时刻捕获的 current_device)：
    //   - 类型匹配（ev_dev 类型 == 事件类型）：直接用 ev_dev，保证「从哪个设备
    //     拦截的就注回哪个设备」；
    //   - 类型不匹配（跨类型映射，如键盘键→鼠标键）：fallback 到「最近同类设备」
    //     (recent_kb_dev / recent_mouse_dev)，由 main loop 维护并初始化为第一个
    //     同类设备。
    // 绝不能用 pipeline.current_device（=最后输入设备）——键鼠交替时它会指向错误类型。
    fn kb_target_dev(reg: &Registry, ev_dev: u32, fallback: u32) -> u32 {
        match reg.get(ev_dev) {
            Some(d) if d.is_keyboard => ev_dev,
            _ => fallback,
        }
    }
    fn mouse_target_dev(reg: &Registry, ev_dev: u32, fallback: u32) -> u32 {
        match reg.get(ev_dev) {
            Some(d) if d.is_mouse => ev_dev,
            _ => fallback,
        }
    }
    fn drain_emit_log_flt(pipeline: &mut PipelineState, fd: &FilterDriver, registry: &Registry, recent_kb_dev: u32, recent_mouse_dev: u32) {
        let mut safety = 0;
        loop {
            let debug_lines: Vec<_> = pipeline.debug_log.drain(..).collect();
            for d in debug_lines { log!("  DBG: {}", d); }

            let events = std::mem::take(&mut pipeline.emit_log);
            if events.is_empty() || safety > 100 { break; }
            safety += 1;
            for event in events {
                // ── 设备路由：目标设备由「触发设备(ev_dev) + 事件类型」共同决定 ──
                // 同类型映射直接用 ev_dev；跨类型回退 0（驱动默认同类设备）。
                // 见 drain_emit_log_flt 上方注释。
                // ── 纯执行函数：发送单个键盘键 ──
                fn send_key_event(fd: &FilterDriver, kn_lower: &str, target_dev: u32, is_down: bool, is_tap: bool) {
                    if let Some(sc) = key_name_to_scancode(kn_lower) {
                        let kind = if is_tap { "TAP" } else if is_down { "DN" } else { "UP" };
                        log!("  {} send(FLT): {} {} dev={}", ts_tag(), kind, kn_lower, target_dev);
                        let ext = needs_e0(kn_lower);
                        let mut flags: u16 = 0;
                        if ext { flags |= 0x02; }
                        if !is_down { flags |= 0x01; }
                        let evt = AnyKeyOutputEvent { make_code: sc, flags, device_id: target_dev };
                        if let Err(e) = fd.send_output(&evt) { log!("  send_output failed: {}", e); }
                        if is_tap {
                            let evt_up = AnyKeyOutputEvent { make_code: sc, flags: flags | 0x01, device_id: target_dev };
                            let _ = fd.send_output(&evt_up);
                        }
                    } else {
                        log!("  ERROR: key '{}' has no scancode — pipeline should have filtered it", kn_lower);
                    }
                }
                // ── 纯执行函数：发送鼠标事件 ──
                fn send_mouse_event(fd: &FilterDriver, name: &str, target_dev: u32, is_down: bool) {
                    use anykey_engine::emit::mouse_name_to_flags;
                    if let Some((mflags, is_wheel)) = mouse_name_to_flags(name, is_down) {
                        log!("  {} send(FLT): MOUSE {} {} dev={}", ts_tag(), if is_down { "DN" } else { "UP" }, name, target_dev);
                        let mut mevt = AnyKeyMouseOutputEvent {
                            device_id: target_dev, flags: 0,
                            button_flags: mflags, button_data: 0,
                            last_x: 0, last_y: 0,
                        };
                        if is_wheel {
                            mevt.button_data = match name.to_lowercase().as_str() {
                                "wheelup" | "wheelright" => 120,
                                "wheeldown" | "wheelleft" => -120,
                                _ => 0,
                            };
                        }
                        if let Err(e) = fd.send_mouse_output(&mevt) { log!("  send_mouse_output failed: {}", e); }
                    } else {
                        log!("  ERROR: mouse name '{}' not recognized — pipeline should have filtered it", name);
                    }
                }
                match event {
                    // ── 键盘键：pipeline 已判断，此处纯执行 ──
                    EmitEvent::Down(ref kn, ev_dev) | EmitEvent::Tap(ref kn, ev_dev) | EmitEvent::Up(ref kn, ev_dev) => {
                        let target_dev = kb_target_dev(registry, ev_dev, recent_kb_dev);
                        if is_layer_key(kn) { continue; }
                        let is_tap = matches!(event, EmitEvent::Tap(_, _));
                        let is_down = matches!(event, EmitEvent::Down(_, _)) || is_tap;
                        let kn_clean = kn.trim_matches(|c: char| c=='{'||c=='}'||c.is_whitespace());
                        let kn_lower = kn_clean.to_lowercase();
                        send_key_event(&fd, &kn_lower, target_dev, is_down, is_tap);
                    }
                    // ── 鼠标键：pipeline 已判断，此处纯执行 ──
                    EmitEvent::MouseDown(ref name, ev_dev) => {
                        send_mouse_event(&fd, name, mouse_target_dev(registry, ev_dev, recent_mouse_dev), true);
                    }
                    EmitEvent::MouseUp(ref name, ev_dev) => {
                        send_mouse_event(&fd, name, mouse_target_dev(registry, ev_dev, recent_mouse_dev), false);
                    }
                    // ── SI 变体（由 leader 发出）：扫描码→SendInput，无扫描码→Unicode ──
                    EmitEvent::TapSI(ref kn, _) | EmitEvent::DownSI(ref kn, _) | EmitEvent::UpSI(ref kn, _) => {
                        let is_tap = matches!(event, EmitEvent::TapSI(_, _));
                        let is_down = matches!(event, EmitEvent::DownSI(_, _)) || is_tap;
                        let kn_clean = kn.trim_matches(|c: char| c=='{'||c=='}'||c.is_whitespace());
                        let kn_lower = kn_clean.to_lowercase();
                        if let Some(sc) = key_name_to_scancode(&kn_lower) {
                            let ext = needs_e0(&kn_lower);
                            log!("  {} send(SI): {} {} (sc=0x{:02X})", ts_tag(), if is_tap{"TAP"}else if is_down{"DN"}else{"UP"}, kn_lower, sc);
                            let sent = send_key_down_sendinput(sc, ext);
                            if sent == 0 {
                                log!("  {} WARNING: SendInput DN '{}' blocked/failed (returned 0, UIPI or security software)", ts_tag(), kn_lower);
                            }
                            if is_tap {
                                let sent_up = send_key_up_sendinput(sc, ext);
                                if sent_up == 0 {
                                    log!("  {} WARNING: SendInput UP '{}' blocked/failed (returned 0, UIPI or security software)", ts_tag(), kn_lower);
                                }
                            }
                        } else {
                            log!("  ERROR: unknown key SI '{}' (no scancode)", kn_lower);
                        }
                    }
                    EmitEvent::Text(ref t, _) => {
                        log!("  {} text: '{}'", ts_tag(), t);
                        send_unicode_text(t);
                    }
                    EmitEvent::Run(ref c, _) => {
                        log!("  {} run: {}", ts_tag(), c);
                        run_shell(c);
                    }
                    EmitEvent::LayerOn(ref l, _) => { log!("  layer ON: {}", l); }
                    EmitEvent::LayerOff(ref l, _) => { log!("  layer OFF: {}", l); }
                    EmitEvent::MouseMove(x, y, ev_dev) => {
                        let target_dev = mouse_target_dev(registry, ev_dev, recent_mouse_dev);
                        log!("  {} send(FLT): MouseMove({}, {}) dev={}", ts_tag(), x, y, target_dev);
                        let mevt = AnyKeyMouseOutputEvent {
                            device_id: target_dev, flags: 0,
                            button_flags: 0, button_data: 0,
                            last_x: x, last_y: y,
                        };
                        if let Err(e) = fd.send_mouse_output(&mevt) {
                            log!("  send_mouse_output(MouseMove) failed: {}", e);
                        }
                    }
                    // ── 通道栅栏【指令，无输出】：睡够前序 FLT 的排空时间，再放行后缀 SI 文本 ──
                    // 时长由 commit 阶段（决策方）按本执行内积压的 FLT 事件数算出，main 只执行，
                    // 不含任何启发式。普通打字不产生 Fence，零影响。
                    // 位置语义：事件序列 = FLT burst → Fence → SI 文本；到此 FLT 已经
                    // send_output 提交给驱动（IOCTL 非阻塞），sleep 让系统排空它们。
                    EmitEvent::Fence(ms) => {
                        log!("  {} FENCE: FLT->SI, sleep {}ms", ts_tag(), ms);
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                    }
                }
            }
        }
    }

    // ── Main loop (event-driven, v0.2: keyboard + mouse) ──
    let sensor = AppSensor::new();
    let mut manager = RuntimeManager::new();
    // 构建 runtime：为默认 app（""）预构建所有设备 map，保证初始可用。
    // app 切换时才构建该 app × 所有设备（切换后有几秒缓冲，不一次构建全部 app）。
    if !all_device_ids.is_empty() {
        manager.build_app_mappings(&pipeline.config, &all_device_ids, "");
        if debug_enabled {
            log!("[runtime] prebuilt mappings: {} entries, {} devices (app='')",
                 manager.mappings.len(), all_device_ids.len());
        }
    }
    let mut current_app = String::new();
    let mut first_input = false;
    let mut last_active_dev: u32 = 0; // 最近产生输入的设备（0 = 尚无输入）
    // 最近同类输入设备 —— 初始化从 registry 取第一个键盘/鼠标，运行时更新。
    // 用于跨类型映射 fallback（如键盘键→鼠标键时，鼠标事件需要发往最近鼠标设备）。
    let mut recent_kb_dev: u32 = registry.descriptors.values()
        .find(|d| d.is_keyboard)
        .map(|d| d.runtime_device_id)
        .unwrap_or(0);
    let mut recent_mouse_dev: u32 = registry.descriptors.values()
        .find(|d| d.is_mouse)
        .map(|d| d.runtime_device_id)
        .unwrap_or(0);
    if debug_enabled {
        log!("[device] recent_kb_dev={} recent_mouse_dev={}", recent_kb_dev, recent_mouse_dev);
    }
    loop {
        pipeline.sync_tick_to_real_time();
        let timeout = pipeline.next_deadline_ms().unwrap_or(1000);

        let (kb_events, ms_events) = match fd.poll_all(timeout) {
            Ok((kb, ms)) => (kb, ms),
            Err(e) => { log!("poll_all error: {}", e); break; }
        };

        pipeline.sync_tick_to_real_time();

        // ── Hotplug is event-driven, NOT polled on a clock. ──
        // The driver sets ANYKEY_FLAG_DEVICE_CHANGED in its status AND
        // signals the SAME input event that just woke this loop (on a
        // PnP add/remove). So we only react when the driver actually
        // reports a change — we never re-scan on a timer. Reading the
        // status auto-clears the one-shot flag in the driver. Guid-bearing
        // rules reconnect via GUID+HWID (stable across replug); guid-less
        // stale rules fall through to `auto_reconnect`. ──
        match fd.get_status() {
            Ok(st) => {
                if st.flags & ANYKEY_FLAG_DEVICE_CHANGED != 0 {
                    log!("[hotplug] driver reported device change — re-matching");
                    rematch(&mut pipeline, &mut manager, &mut matcher, &mut registry, &fd, debug_enabled);
                }
            }
            Err(e) => log!("[hotplug] get_status failed: {}", e),
        }

        // Process expired timers
        let expired = pipeline.pop_expired_timers();

        // ── App-aware switch detection ──
        // sensor 已按 HWND 去重（event-driven + 500ms watchdog），这里只处理「确实变化」。
        if let Some(info) = sensor.try_recv() {
            // 进程名统一小写：Windows 前台进程名大小写不定（Revit.exe vs revit.exe），
            // config appAware 的 key 也是小写 —— 必须归一化否则 app 覆盖匹配不到。
            let app = info.process.trim().to_lowercase();
            // 注意：空进程名（桌面/无法解析的窗口）也必须切回默认映射，
            // 否则切到桌面后仍残留上一个 app 的 mapping（旧代码 !app.is_empty() 的 bug）。
            if app != current_app {
                // 用有效设备：优先最近输入设备，其次当前设备（启动时默认 1 无效）。
                let dev = if last_active_dev != 0 { last_active_dev } else { pipeline.current_device };
                log!("[app] switch gen={} hwnd=0x{:X} pid={}: '{}' -> '{}' (device={} input_dev={})",
                     info.generation, info.hwnd as usize, info.pid, current_app, app, dev, last_active_dev);
                current_app = app.clone();
                pipeline.current_app = app.clone();
                // 切换后一般有几秒缓冲：此时为「当前 app × 所有设备」构建 map（按需，不一次全建）
                if !all_device_ids.is_empty() {
                    manager.build_app_mappings(&pipeline.config, &all_device_ids, &app);
                }
                // 查缓存（零构建）；未命中回退设备默认
                pipeline.mapping = manager.lookup_mapping(dev, &app)
                    .or_else(|| manager.lookup_mapping(dev, ""))
                    .unwrap_or_else(|| manager.get_mapping(&pipeline.config, dev, &app));
                if debug_enabled {
                    dump_mapping(&pipeline.mapping, dev, &app);
                }
            }
        }
        for entry in expired {
            log!("timer: {:?} key={}", entry.kind, entry.key);
            match entry.kind {
                TimerKind::Hold        => pipeline.hold_timer(&entry.key),
                TimerKind::DoubleTap   => pipeline.dt_timer(&entry.key),
                TimerKind::DoubleHold  => pipeline.dh_timer(&entry.key),
                TimerKind::ComboTimeout => pipeline.single_key_timeout(&entry.key),
                TimerKind::SleepTimer => pipeline.fire_sleep_timer(&entry),
                TimerKind::LeaderTimeout => pipeline.leader_timeout(),
            }
            drain_emit_log_flt(&mut pipeline, &fd, &registry, recent_kb_dev, recent_mouse_dev);
        }

        // Process keyboard input events
        for evt in &kb_events {
            // Skip system-level scancodes (0xFF = keyboard reset/BAT completion signal)
            if evt.make_code == 0xFF {
                continue;
            }

            let is_down = (evt.flags & ANYKEY_KEY_BREAK) == 0;

            let key_name_flt = input_name(evt.make_code, evt.flags);
            log!("{} in: {} {}", ts_tag(), if is_down { "DN" } else { "UP" }, key_name_flt);

            pipeline.current_device = evt.device_id;
            last_active_dev = evt.device_id;
            recent_kb_dev = evt.device_id;

            // 设备变化（键鼠交替/首次输入）→ 换对应设备的预构建 mapping（只查不建）。
            // 未命中（未预构建组合）→ 保持现有 mapping，绝不在此构建（避免按键延迟）。
            if let Some(m) = manager.lookup_mapping(pipeline.current_device, &current_app) {
                pipeline.mapping = m;
            } else if let Some(m) = manager.lookup_mapping(pipeline.current_device, "") {
                pipeline.mapping = m;
            }
            if is_down {
                pipeline.key_down(&key_name_flt);
            } else {
                pipeline.key_up(&key_name_flt);
            }
        }

        // Process mouse input events (v0.2 — routed to pipeline)
        let mut last_abs: Option<(i32, i32)> = None;
        for evt in &ms_events {
            recent_mouse_dev = evt.device_id;
            let is_absolute = (evt.flags & 0x01) != 0;

            let (dx, dy) = if is_absolute {
                let delta = if let Some((lx, ly)) = last_abs {
                    (evt.last_x - lx, evt.last_y - ly)
                } else { (0, 0) };
                last_abs = Some((evt.last_x, evt.last_y));
                delta
            } else {
                (evt.last_x, evt.last_y)
            };

            // Button / wheel → pipeline (named like keyboard keys)
            // v0.4.x: translate merged packets into single-edge events first,
            // so multi-bit masks (e.g. LEFT_DOWN|RIGHT_UP) are never dropped.
            for (name, is_down) in mouse_translator.translate(evt) {
                let abs_tag = if is_absolute { "abs" } else { "rel" };
                log!("mouse: dev={} btn=0x{:04X} flg=0x{:02X}({}) {} {} dx={} dy={} (raw x={} y={})",
                     evt.device_id, evt.button_flags, evt.flags, abs_tag,
                     if is_down { "DN" } else { "UP" }, name,
                     dx, dy, evt.last_x, evt.last_y);

                pipeline.current_device = evt.device_id;
                if is_down {
                    pipeline.key_down(name);
                    // Wheel events are instantaneous: follow with key_up immediately
                    if name.starts_with("Wheel") {
                        pipeline.key_up(name);
                    }
                } else {
                    pipeline.key_up(name);
                }
            }
        }

        let has_events = !kb_events.is_empty() || !ms_events.is_empty();
        if has_events || !first_input {
            drain_emit_log_flt(&mut pipeline, &fd, &registry, recent_kb_dev, recent_mouse_dev);
            if has_events { first_input = true; }
        }
    }

    let _ = fd.set_device_intercept(0, false); // v0.4: disable all on shutdown
}

/// Send Unicode text via Windows SendInput (shared by both backends).
fn send_unicode_text(t: &str) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let cap = t.chars().count().saturating_mul(2);
    let mut inputs: Vec<INPUT> = Vec::with_capacity(cap);
    for ch in t.chars() {
        let mut buf = [0u16; 2];
        for u in ch.encode_utf16(&mut buf) {
            let code = *u;
            let mut down: INPUT = unsafe { std::mem::zeroed() };
            down.r#type = INPUT_KEYBOARD;
            down.Anonymous.ki = KEYBDINPUT {
                wVk: 0, wScan: code, dwFlags: KEYEVENTF_UNICODE, time: 0, dwExtraInfo: 0,
            };
            let mut up: INPUT = unsafe { std::mem::zeroed() };
            up.r#type = INPUT_KEYBOARD;
            up.Anonymous.ki = KEYBDINPUT {
                wVk: 0, wScan: code, dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP, time: 0, dwExtraInfo: 0,
            };
            inputs.push(down);
            inputs.push(up);
        }
    }
    if !inputs.is_empty() {
        unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32); }
    }
}

/// Run an external command via ShellExecuteW (shared by both backends).
pub(crate) fn run_shell(cmd: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let (program, params) = split_program_and_args(cmd);
    let wide_program: Vec<u16> = program.encode_utf16().chain(Some(0)).collect();
    let wide_params: Vec<u16> = params.encode_utf16().chain(Some(0)).collect();
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(), verb.as_ptr(),
            wide_program.as_ptr(),
            if params.is_empty() { std::ptr::null() } else { wide_params.as_ptr() },
            std::ptr::null(), SW_SHOWNORMAL,
        );
    }
}

/// 把命令行拆成 (程序, 参数)。
/// 规则：
/// - 以引号开头：引号内为程序（支持含空格路径），其后为参数；
/// - 否则：仅当「首个空白前的 token 看起来是可执行文件」(.exe/.bat/.cmd/.com/.msi/.ps1 等)
///   才拆成 程序+参数；否则把整串当作路径/文档整体打开（lpFile），不拆分。
///   这样 RUN: 既能正确传递程序参数（如 rundll32.exe + dll），
///   也能直接打开含空格/中文的文件夹或文档（整串即 lpFile）。
pub(crate) fn split_program_and_args(cmd: &str) -> (String, String) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return (String::new(), String::new());
    }
    // 引号包裹的程序路径：引号内是程序，其后为参数
    if cmd.starts_with('"') {
        if let Some(end) = cmd[1..].find('"') {
            let program = cmd[1..=end].to_string();
            let params = cmd[end + 2..].trim_start().to_string();
            return (program, params);
        }
        // 引号未闭合：退化为整串当程序
        return (cmd.to_string(), String::new());
    }
    // 首个空白处尝试拆分，但仅在「首个 token 像可执行文件」时才拆；
    // 否则整串当作路径/文档打开（含空格/中文路径不被误拆）。
    if let Some(sp) = cmd.find(char::is_whitespace) {
        let first = &cmd[..sp];
        if is_executable_name(first) {
            return (first.to_string(), cmd[sp + 1..].trim_start().to_string());
        }
        // 不是可执行文件：整串即路径，不拆分
        return (cmd.to_string(), String::new());
    }
    (cmd.to_string(), String::new())
}

/// 判断一个名字是否像「可执行文件」（用于决定是否把 RUN 命令拆成 程序+参数）。
fn is_executable_name(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.ends_with(".exe") || lower.ends_with(".bat") || lower.ends_with(".cmd")
        || lower.ends_with(".com") || lower.ends_with(".msi") || lower.ends_with(".msc")
        || lower.ends_with(".ps1") || lower.ends_with(".vbs") || lower.ends_with(".js")
}

// ── Build matcher rules from the live config (one DeviceRule per
// enabled subscription). Used by `rematch` at startup and on hotplug. ──
fn build_rules(config: &Config) -> HashMap<String, DeviceRule> {
    let mut rules = HashMap::new();
    for dev in &config.subscribed_devices {
        if !dev.enabled { continue; }
        rules.insert(dev.alias.clone(), DeviceRule {
            runtime_device_id: dev.runtime_device_id,
            guid:               dev.guid.clone(),
            vid:                u16::from_str_radix(&dev.vid, 16).unwrap_or(0),
            pid:                u16::from_str_radix(&dev.pid, 16).unwrap_or(0),
            kind:               dev.kind.clone(),
            hardware_id:        dev.hardware_id.clone(),
        });
    }
    rules
}

/// Re-match every subscription against the current device set and
/// rebuild per-device contexts. Mirrors the startup path: each rule
/// resolves independently and device ids are collected into a
/// HashSet — no alias-based collision as with `match_all`.
fn rematch(
    pipeline: &mut PipelineState,
    manager:  &mut RuntimeManager,
    matcher:  &mut Matcher,
    registry: &mut Registry,
    fd:       &FilterDriver,
    debug:    bool,
) {
    let _ = registry.init(fd);
    let descriptors: Vec<DeviceDescriptor> = registry.descriptors.values().cloned().collect();
    if debug {
        log!("[dev-scan] {} device(s) re-enumerated:", descriptors.len());
        for d in &descriptors {
            log!("  [dev] id={} name='{}' guid={:?} vid={:04X} pid={:04X} kbd={} mouse={} hwid='{}'",
                 d.runtime_device_id, d.friendly_name, d.container_id,
                 d.vendor_id, d.product_id, d.is_keyboard, d.is_mouse, d.hardware_id);
        }
    }
    matcher.load_rules(build_rules(&pipeline.config));

    // Mirror the startup path: resolve each rule independently into a HashSet.
    // This avoids the alias-collision problem that `match_all` has with
    // composite devices (keyboard+mouse sharing the same alias).
    let mut subscribed_ids: HashSet<u32> = HashSet::new();
    for dev in &pipeline.config.subscribed_devices {
        if !dev.enabled {
            if debug {
                log!("  [sub] SKIP disabled: alias='{}' guid={:?} rid={:?} kind='{}'",
                     dev.alias, dev.guid, dev.runtime_device_id, dev.kind);
            }
            continue;
        }
        let rule = DeviceRule {
            runtime_device_id: dev.runtime_device_id,
            guid:               dev.guid.clone(),
            vid:                u16::from_str_radix(&dev.vid, 16).unwrap_or(0),
            pid:                u16::from_str_radix(&dev.pid, 16).unwrap_or(0),
            kind:               dev.kind.clone(),
            hardware_id:        dev.hardware_id.clone(),
        };
        if let Some(id) = matcher.resolve(&descriptors, &rule) {
            let name = descriptors.iter()
                .find(|d| d.runtime_device_id == id)
                .map(|d| d.friendly_name.as_str())
                .unwrap_or("?");
            log!("  [sub] alias='{}' guid={:?} vid={:04X} pid={:04X} kind='{}' rid={:?} => MATCH device_id={} name='{}'",
                 dev.alias, dev.guid, rule.vid, rule.pid, dev.kind, dev.runtime_device_id, id, name);
            subscribed_ids.insert(id);
        } else {
            log!("  [sub] alias='{}' guid={:?} vid={:04X} pid={:04X} kind='{}' rid={:?} => NO MATCH (device not resolved)",
                 dev.alias, dev.guid, rule.vid, rule.pid, dev.kind, dev.runtime_device_id);
        }
        for line in matcher.debug_lines.drain(..) { log!("    {}", line); }
    }

    if !descriptors.is_empty() {
        let ids: Vec<u32> = descriptors.iter().map(|d| d.runtime_device_id).collect();
        // 构建 runtime：为默认 app 预构建所有设备 mapping（app 切换时会按需构建）
        manager.build_app_mappings(&pipeline.config, &ids, "");
        let first_device = *ids.first().unwrap_or(&1);
        pipeline.mapping = manager.lookup_mapping(first_device, "")
            .unwrap_or_else(|| manager.get_mapping(&pipeline.config, first_device, ""));
        if debug {
            dump_mapping(&pipeline.mapping, first_device, "");
        }
        log!("Multi-device [rematch]: {} device(s), {} subscribed, pipeline reloaded (mappings={})",
             ids.len(), subscribed_ids.len(), manager.mappings.len());
    }

    // v0.4: per-device intercept — update driver for matched devices
    // First disable all, then enable only subscribed ones
    let _ = fd.set_device_intercept(0, false);
    for &id in &subscribed_ids {
        match fd.set_device_intercept(id, true) {
            Ok(_) => log!("[rematch] device {} → INTERCEPT ON", id),
            Err(e) => log!("[rematch] device {} FAILED: {}", id, e),
        }
    }
}

/// 调试：打印 DeviceMapping 的完整结构（combo map + tap_dance map + leader）。
/// 在 runtime 构建（app 切换 / 设备初始化 / rematch）时调用，用于排查
/// 「应用感知映射不正确」类问题——直接看最终生效的 map。
fn dump_mapping(mapping: &Arc<DeviceMapping>, device: u32, app: &str) {
    log!("===== MAPPING device={} app='{}' =====", device, app);
    // combo map: [layer] k + p -> out
    let mut layers: Vec<&String> = mapping.combo.map.keys().collect();
    layers.sort();
    for layer in layers {
        if let Some(lm) = mapping.combo.map.get(layer) {
            let mut keys: Vec<&String> = lm.keys().collect();
            keys.sort();
            for k in keys {
                if let Some(partners) = lm.get(k) {
                    let mut ps: Vec<(&String, &String)> = partners.iter().collect();
                    ps.sort_by(|a, b| a.0.cmp(b.0));
                    for (p, out) in ps {
                        log!("  [combo] [{}] {} + {} -> {}", layer, k, p, out);
                    }
                }
            }
        }
    }
    // tap_dance map: [layer] phys: tap/hold/dt/dh
    let mut td_layers: Vec<&String> = mapping.tap_dance.keys().collect();
    td_layers.sort();
    for layer in td_layers {
        if let Some(km) = mapping.tap_dance.get(layer) {
            let mut keys: Vec<&String> = km.keys().collect();
            keys.sort();
            for k in keys {
                if let Some(e) = km.get(k) {
                    log!("  [td] [{}] {} tap='{}' hold='{}' dt='{}' dh='{}'",
                         layer, k, e.tap, e.hold, e.double_tap, e.double_hold);
                }
            }
        }
    }
    // leader
    log!("  [leader] sequences={} timeout={}", mapping.leader.sequences.len(), mapping.leader.timeout_ms);
    log!("===== MAPPING END =====");
}

#[cfg(test)]
mod run_shell_tests {
    use crate::split_program_and_args;
    #[test]
    fn split_run_with_args() {
        let (p, a) = split_program_and_args("rundll32.exe powrprof.dll,SetSuspendState 0,1,0");
        assert_eq!(p, "rundll32.exe");
        assert_eq!(a, "powrprof.dll,SetSuspendState 0,1,0");
    }
    #[test]
    fn split_run_no_args() {
        let (p, a) = split_program_and_args("notepad.exe");
        assert_eq!(p, "notepad.exe");
        assert!(a.is_empty());
    }
    #[test]
    fn split_run_quoted_path() {
        let (p, a) = split_program_and_args(r#""C:/Program Files/foo/bar.exe" arg1 arg2"#);
        assert_eq!(p, r"C:/Program Files/foo/bar.exe");
        assert_eq!(a, "arg1 arg2");
    }
    #[test]
    fn split_run_chinese_path_keeps_whole() {
        // 含空格/中文的文件夹路径：不应被空格拆开，整串当作 lpFile 打开
        let (p, a) = split_program_and_args("Z:\\P20230327 -悉尼滨水别墅项目");
        assert_eq!(p, "Z:\\P20230327 -悉尼滨水别墅项目");
        assert!(a.is_empty());
    }
    #[test]
    fn split_run_exe_with_args_splits() {
        // rundll32 这类可执行文件 + 参数：仍应拆分
        let (p, a) = split_program_and_args("rundll32.exe powrprof.dll,SetSuspendState 0,1,0");
        assert_eq!(p, "rundll32.exe");
        assert_eq!(a, "powrprof.dll,SetSuspendState 0,1,0");
    }

    // ── 回归：config 规范化后的全名（pagedown/pageup）必须能被识别 ──
    use super::{key_name_to_scancode, needs_e0};
    #[test]
    fn pagedown_pageup_canonical_names_resolve() {
        // 引擎不再输出归一化（由 GUI 在保存时完成），此处仅为格式说明：
        // 因此 scancode 表必须认识全名，否则发送时报 UNKNOWN KEY: 'pagedown'
        assert_eq!(key_name_to_scancode("pagedown"), Some(0x51), "pagedown → 0x51");
        assert_eq!(key_name_to_scancode("pageup"), Some(0x49), "pageup → 0x49");
        // 短名仍应可用
        assert_eq!(key_name_to_scancode("pgdn"), Some(0x51));
        assert_eq!(key_name_to_scancode("pgup"), Some(0x49));
        // PageUp/PageDown 属于扩展键，必须带 E0 前缀
        assert!(needs_e0("pagedown"), "pagedown needs E0");
        assert!(needs_e0("pageup"), "pageup needs E0");
        assert!(needs_e0("pgdn"));
        assert!(needs_e0("pgup"));
    }
}
