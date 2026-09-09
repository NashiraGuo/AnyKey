// Integration test: merged mouse packet → MouseEventTranslator → pipeline combo.
//
// The old bug: a merged packet (e.g. LEFT_DOWN|RIGHT_UP = 0x0009) was silently
// dropped by the exact-match naming in the main loop, so the combo's partner
// release never arrived and the combo output stayed held (deadlock).
//
// With the translator in the input layer, every valid edge is delivered as a
// single-bit event, so the left+right combo must fire Down "1" and then cleanly
// release Up "1" when the (merged) partner release arrives.

use anykey_engine::config::Config;
use anykey_engine::state::*;
use anykey_engine::filter_driver::{
    MouseEventTranslator, AnyKeyMouseEvent,
    MOUSE_LEFT_BUTTON_DOWN, MOUSE_LEFT_BUTTON_UP,
    MOUSE_RIGHT_BUTTON_DOWN, MOUSE_RIGHT_BUTTON_UP,
};

fn mk(dev: u32, button_flags: u16) -> AnyKeyMouseEvent {
    AnyKeyMouseEvent {
        device_id: dev,
        flags: 0,
        button_flags,
        button_data: 0,
        last_x: 0,
        last_y: 0,
        extra_info: 0,
    }
}

#[test]
fn merged_packet_combo_fires_and_releases() {
    let json = r#"{
      "comboTime": 200,
      "comboMap": [
        {"key1":"mouseleft","key2":"mouseright","output":"{1}","layer":"base"}
      ],
      "tapDance": {"base": {}},
      "subscribed_devices": []
    }"#;
    let config: Config = serde_json::from_str(json).unwrap();

    let mut pipeline = PipelineState::new(config);
    pipeline.debug_enabled = true;
    let mut tr = MouseEventTranslator::new();

    // Raw driver packet stream for: press L, press R, release R (as a MERGED
    // packet that also re-asserts the held L), release L.
    let seq = [
        MOUSE_LEFT_BUTTON_DOWN,                         // press left
        MOUSE_RIGHT_BUTTON_DOWN,                        // press right -> combo fires Down "1"
        MOUSE_LEFT_BUTTON_DOWN | MOUSE_RIGHT_BUTTON_UP, // merged 0x0009: phantom L re-assert + real R up
        MOUSE_LEFT_BUTTON_UP,                           // release left -> partner release -> Up "1"
    ];
    for flags in seq {
        for (name, is_down) in tr.translate(&mk(1, flags)) {
            if is_down {
                pipeline.key_down(name);
            } else {
                pipeline.key_up(name);
            }
        }
    }

    println!("=== EMIT LOG (merged packet flow) ===");
    for e in &pipeline.emit_log {
        println!("  {:?}", e);
    }

    let downs: Vec<&EmitEvent> = pipeline
        .emit_log
        .iter()
        .filter(|e| matches!(e, EmitEvent::Down(k, _) | EmitEvent::MouseDown(k, _) if k.eq_ignore_ascii_case("1")))
        .collect();
    let ups: Vec<&EmitEvent> = pipeline
        .emit_log
        .iter()
        .filter(|e| matches!(e, EmitEvent::Up(k, _) | EmitEvent::MouseUp(k, _) if k.eq_ignore_ascii_case("1")))
        .collect();

    println!("downs={} ups={}", downs.len(), ups.len());

    assert_eq!(downs.len(), 1, "combo should fire exactly one Down \"1\"");
    assert_eq!(ups.len(), 1, "combo must release exactly one Up \"1\" (no deadlock from merged packet)");

    let down_idx = pipeline
        .emit_log
        .iter()
        .position(|e| matches!(e, EmitEvent::Down(k, _) | EmitEvent::MouseDown(k, _) if k.eq_ignore_ascii_case("1")))
        .unwrap();
    let up_idx = pipeline
        .emit_log
        .iter()
        .position(|e| matches!(e, EmitEvent::Up(k, _) | EmitEvent::MouseUp(k, _) if k.eq_ignore_ascii_case("1")))
        .unwrap();
    assert!(down_idx < up_idx, "Down \"1\" must come before Up \"1\"");
}
