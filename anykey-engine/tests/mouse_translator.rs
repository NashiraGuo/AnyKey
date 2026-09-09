// MouseEventTranslator unit tests — merged-packet splitting + spurious-edge dropping.
//
// The driver passes raw MOUSE_INPUT_DATA.ButtonFlags bitmasks through verbatim.
// A single packet may carry several edges (e.g. LEFT_DOWN|RIGHT_UP = 0x0009).
// The translator must emit EVERY valid edge (fixed order L→R→M→X1→X2, down
// before up) and only drop self-contradictory edges.

use anykey_engine::filter_driver::{
    MouseEventTranslator, AnyKeyMouseEvent,
    MOUSE_LEFT_BUTTON_DOWN, MOUSE_LEFT_BUTTON_UP,
    MOUSE_RIGHT_BUTTON_DOWN, MOUSE_RIGHT_BUTTON_UP,
    MOUSE_MIDDLE_BUTTON_DOWN, MOUSE_WHEEL, MOUSE_HWHEEL,
};

fn ev(dev: u32, button_flags: u16) -> AnyKeyMouseEvent {
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

fn names(v: Vec<(&str, bool)>) -> Vec<(String, bool)> {
    v.into_iter().map(|(n, d)| (n.to_string(), d)).collect()
}

#[test]
fn single_click_left() {
    let mut t = MouseEventTranslator::new();
    assert_eq!(
        names(t.translate(&ev(1, MOUSE_LEFT_BUTTON_DOWN))),
        vec![("MouseLeft".into(), true)]
    );
    assert_eq!(
        names(t.translate(&ev(1, MOUSE_LEFT_BUTTON_UP))),
        vec![("MouseLeft".into(), false)]
    );
}

#[test]
fn two_buttons_same_packet_both_emitted_in_order() {
    // 0x0005 = LEFT_DOWN|RIGHT_DOWN: both valid -> both emitted, L before R.
    let mut t = MouseEventTranslator::new();
    let f = MOUSE_LEFT_BUTTON_DOWN | MOUSE_RIGHT_BUTTON_DOWN;
    assert_eq!(
        names(t.translate(&ev(1, f))),
        vec![("MouseLeft".into(), true), ("MouseRight".into(), true)]
    );
}

#[test]
fn three_buttons_same_packet_all_emitted() {
    // 0x0015 = L|R|M down: all three valid edges must be delivered, none dropped.
    let mut t = MouseEventTranslator::new();
    let f = MOUSE_LEFT_BUTTON_DOWN | MOUSE_RIGHT_BUTTON_DOWN | MOUSE_MIDDLE_BUTTON_DOWN;
    assert_eq!(names(t.translate(&ev(1, f))).len(), 3);
}

#[test]
fn merged_release_recovers_partner_up() {
    // Hold L+R, then release R while firmware re-asserts the held L:
    // packet = LEFT_DOWN|RIGHT_UP (0x0009). The phantom L-down is dropped
    // (L already pressed) and the real Right-up is emitted — this is the
    // exact packet that used to deadlock the left+right combo.
    let mut t = MouseEventTranslator::new();
    t.translate(&ev(1, MOUSE_LEFT_BUTTON_DOWN));
    t.translate(&ev(1, MOUSE_RIGHT_BUTTON_DOWN));
    let f = MOUSE_LEFT_BUTTON_DOWN | MOUSE_RIGHT_BUTTON_UP;
    assert_eq!(
        names(t.translate(&ev(1, f))),
        vec![("MouseRight".into(), false)]
    );
}

#[test]
fn spurious_up_dropped() {
    let mut t = MouseEventTranslator::new();
    assert!(t.translate(&ev(1, MOUSE_LEFT_BUTTON_UP)).is_empty());
}

#[test]
fn double_down_dropped() {
    let mut t = MouseEventTranslator::new();
    t.translate(&ev(1, MOUSE_LEFT_BUTTON_DOWN));
    assert!(t.translate(&ev(1, MOUSE_LEFT_BUTTON_DOWN)).is_empty());
}

#[test]
fn same_packet_down_up_is_a_click() {
    // 0x0003 = LEFT_DOWN|LEFT_UP in one packet: down then up, one click.
    let mut t = MouseEventTranslator::new();
    let f = MOUSE_LEFT_BUTTON_DOWN | MOUSE_LEFT_BUTTON_UP;
    assert_eq!(
        names(t.translate(&ev(1, f))),
        vec![("MouseLeft".into(), true), ("MouseLeft".into(), false)]
    );
}

#[test]
fn wheel_passthrough() {
    let mut t = MouseEventTranslator::new();

    let mut e = ev(1, MOUSE_WHEEL);
    e.button_data = 120;
    assert_eq!(names(t.translate(&e)), vec![("WheelUp".into(), true)]);

    let mut e = ev(1, MOUSE_HWHEEL);
    e.button_data = -120;
    assert_eq!(names(t.translate(&e)), vec![("WheelLeft".into(), true)]);
}

#[test]
fn per_device_state_isolated() {
    let mut t = MouseEventTranslator::new();
    t.translate(&ev(1, MOUSE_LEFT_BUTTON_DOWN)); // dev 1 presses L
    // dev 2 sees the same DOWN as a fresh press, NOT a spurious re-assertion.
    let v = t.translate(&ev(2, MOUSE_LEFT_BUTTON_DOWN));
    assert_eq!(names(v), vec![("MouseLeft".into(), true)]);
}

#[test]
fn reset_clears_all_state() {
    let mut t = MouseEventTranslator::new();
    t.translate(&ev(1, MOUSE_LEFT_BUTTON_DOWN));
    t.reset();
    // After reset, an UP for dev 1 is spurious again (nothing pressed).
    assert!(t.translate(&ev(1, MOUSE_LEFT_BUTTON_UP)).is_empty());
}
