/// Control test for the "mouse left+right combo" path.
///
/// This replays the CORRECT physical event stream that the engine SHOULD receive
/// when the user presses left+right and then releases both:
///     LeftDown, RightDown, RightUp, LeftUp
///
/// If the input layer (driver) delivered this exact stream (i.e. the right-up is
/// NOT mislabeled), the combo must fire its output and then cleanly release it
/// once the second (partner) key goes up — NO deadlock, NO stuck key.
///
/// This isolates the bug to the input layer: the engine's combo logic itself is
/// correct given well-formed edge events. The deadlock in
/// `combo_mouse_mislabel_repro.rs` is solely caused by the mislabeled right-up.
///
/// NOTE: after the engine-side combo partner-release watchdog (see fix plan) is
/// implemented, the mislabel repro must instead assert RECOVERY (Up "1" emitted),
/// because the watchdog would also release this combo. The present test keeps
/// validating the well-formed path independently.

use anykey_engine::config::Config;
use anykey_engine::state::*;

#[test]
fn control_mouse_combo_correct_release_no_deadlock() {
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

    // Correct physical stream: both down, then both up.
    pipeline.key_down("mouseleft"); // real
    pipeline.key_down("mouseright"); // real -> combo armed, emits Down "1"
    pipeline.key_up("mouseright"); // real right-up (first release -> silenced, wait for partner)
    pipeline.key_up("mouseleft"); // real left-up (partner release -> emit Up "1")

    println!("=== EMIT LOG (correct flow) ===");
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
    assert_eq!(ups.len(), 1, "combo should release exactly one Up \"1\" (no deadlock)");

    // The Down must precede the Up.
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
