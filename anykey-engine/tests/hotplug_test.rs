// ── Hotplug / rematch regression tests ──
// Integration tests: mirror the HashSet-based resolve pattern that
// `rematch` (main.rs) and the startup path both use. Verifies that
// composite devices with shared aliases do NOT suffer alias collision.
//
// These are integration tests (in tests/), not unit tests inside
// matcher.rs: they exercise the public Matcher API exactly as the
// engine's main loop does.

use anykey_engine::matcher::{Matcher, DeviceRule};
use anykey_engine::registry::DeviceDescriptor;
use std::collections::HashSet;

// ── Shared fixtures ──

/// Composite device: keyboard + mouse sub-devices sharing the same
/// ContainerID, modelling a real ROG STRIX SCOPE II RX after hotplug.
fn composite_descriptors() -> Vec<DeviceDescriptor> {
    let guid = Some("{A1B2C3D4-A1B2-C3D4-A1B2-C3D4A1B2C3D4}".into());
    vec![
        DeviceDescriptor {
            runtime_device_id: 10,
            container_id: guid.clone(),
            hardware_id: "HID\\VID_0B05&PID_1AB5&REV_0115&MI_00".into(),
            friendly_name: "ROG Keyboard (kbd)".into(),
            vendor_id: 0x0B05, product_id: 0x1AB5,
            alias: None,
            is_keyboard: true, is_mouse: false, has_serial: true, flags: 0,
        },
        DeviceDescriptor {
            runtime_device_id: 11,
            container_id: guid.clone(),
            hardware_id: "HID\\VID_0B05&PID_1AB5&REV_0115&MI_02&Col04".into(),
            friendly_name: "ROG STRIX SCOPE II RX (media)".into(),
            vendor_id: 0x0B05, product_id: 0x1AB5,
            alias: None,
            // Mirror real-world driver misclassification: media interface
            // flagged as mouse by AnyKey_DetectDeviceType in the driver.
            is_keyboard: false, is_mouse: true, has_serial: true, flags: 0,
        },
        DeviceDescriptor {
            runtime_device_id: 12,
            container_id: None,
            hardware_id: "HID\\VID_046D&PID_C232".into(),
            friendly_name: "Logitech Keyboard (standalone)".into(),
            vendor_id: 0x046D, product_id: 0xC232,
            alias: None,
            is_keyboard: true, is_mouse: false, has_serial: true, flags: 0,
        },
    ]
}

// ── Tests ──

#[test]
fn test_rematch_composite_device_both_kinds_matched() {
    // Scenario: ROG keyboard replugged. Engine rematch must match both
    // the keyboard rule (→ dev 10) and the mouse rule (→ dev 11) into
    // subscribed_ids, NOT collide on a shared alias like the old match_all did.
    let devs = composite_descriptors();
    let mut m = Matcher::new();

    let guid = "{A1B2C3D4-A1B2-C3D4-A1B2-C3D4A1B2C3D4}";
    let kbd_rule = DeviceRule {
        guid: Some(guid.into()), vid: 0x0B05, pid: 0x1AB5,
        kind: "keyboard".into(), runtime_device_id: None,
        hardware_id: Some("HID\\VID_0B05&PID_1AB5&REV_0115&MI_00".into()),
    };
    let mouse_rule = DeviceRule {
        guid: Some(guid.into()), vid: 0x0B05, pid: 0x1AB5,
        kind: "mouse".into(), runtime_device_id: None,
        hardware_id: Some("HID\\VID_0B05&PID_1AB5&REV_0115&MI_02&Col04".into()),
    };

    // Mirror the rematch/startup pattern: resolve each rule into a HashSet.
    let mut subscribed: HashSet<u32> = HashSet::new();
    if let Some(id) = m.resolve(&devs, &kbd_rule) { subscribed.insert(id); }
    if let Some(id) = m.resolve(&devs, &mouse_rule) { subscribed.insert(id); }

    assert!(subscribed.contains(&10), "keyboard rule (kind=keyboard hwid=MI_00) must match device 10");
    assert!(subscribed.contains(&11), "mouse rule (kind=mouse hwid=MI_02) must match device 11");
    assert_eq!(subscribed.len(), 2,
        "composite device: both keyboard and mouse must be subscribed (no alias collision)");
}

#[test]
fn test_rematch_device_gone_no_match() {
    // Device 12 is a standalone keyboard. After it's unplugged, the rule
    // matches nothing → resolve returns None → not added to subscribed_ids.
    let devs = composite_descriptors();
    let mut m = Matcher::new();

    let gone_rule = DeviceRule {
        guid: None, vid: 0xDEAD, pid: 0xBEEF,
        kind: "keyboard".into(), runtime_device_id: None, hardware_id: None,
    };
    let result = m.resolve(&devs, &gone_rule);
    assert!(result.is_none(), "rule for unplugged device must return None");
}

#[test]
fn test_rematch_guidless_stale_id_auto_reconnect_with_kind_filter() {
    // Regression: Tier4 auto_reconnect must respect `kind`. A keyboard
    // rule must NOT match a mouse-only device with the same VID/PID.
    let devs = vec![
        DeviceDescriptor {
            runtime_device_id: 20,
            container_id: None,
            hardware_id: "HID\\VID_046D&PID_C539&...".into(),
            friendly_name: "Logitech Mouse (no-serial)".into(),
            vendor_id: 0x046D, product_id: 0xC539,
            alias: None,
            is_keyboard: false, is_mouse: true, has_serial: false, flags: 0,
        },
    ];
    let mut m = Matcher::new();
    // Keyboard rule with stale RID, same VID/PID as the mouse above.
    // auto_reconnect must NOT return device 20 because kind doesn't match.
    let kbd_rule = DeviceRule {
        guid: None,
        vid: 0x046D, pid: 0xC539, kind: "keyboard".into(),
        runtime_device_id: Some(99),
        hardware_id: None,
    };
    let result = m.resolve(&devs, &kbd_rule);
    assert!(result.is_none(),
        "Tier4 auto_reconnect: keyboard rule must NOT match mouse-only device (kind filter)");
}
