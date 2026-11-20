/// AnyKey engine — TapDone 结算未出栈导致 waiting_stack 残留的回归测试
///
/// 用户场景：space 配 hold={fn1} + dt={f2}（switch 键且带 double_tap）。
/// 单击 space 后 dt_timer 到期结算为单 tap，但 flush_tapdone_as_tap 原先
/// 没有出栈，space 以 Finished 状态永久留在 waiting_stack。此后当前层
/// 无 TD 定义的键（如 mouseright）被 try_defer_layer_interrupt 错误延迟：
/// Down 被推迟到 keyup 才发，表现为「右键只能点击、不能按住」。
///
/// 修复：flush_tapdone_as_tap 在 reenter_up 前登记 force_hold_up，
/// 由 up4 → force_hold_execute 统一出栈。

use anykey_engine::config::*;
use anykey_engine::state::*;
use serde_json::json;

fn build_config() -> Config {
    let cfg = json!({
        "comboTime": 200,
        "tapDance": {
            "baseLayer": {
                "space": { "tap": "{space}", "hold": "{fn1}", "dt": "{f2}", "dtt": "250" },
                "mouseright": { "tap": "{mousemiddle}" }
            },
            "fn1": {},
            "holdTerm": 200,
            "doubleTapTerm": 250,
            "doubleHoldTerm": 150
        }
    });
    serde_json::from_value(cfg).unwrap()
}

fn on_stack(p: &PipelineState, key: &str) -> bool {
    p.state.defer.waiting_stack.iter().any(|(k, _)| k == key)
}

fn emitted(p: &PipelineState, key: &str) -> bool {
    p.emit_log.iter().any(|e| matches!(e,
        EmitEvent::Down(k, _) | EmitEvent::MouseDown(k, _) if k == key))
}

fn emitted_up(p: &PipelineState, key: &str) -> bool {
    p.emit_log.iter().any(|e| matches!(e,
        EmitEvent::Up(k, _) | EmitEvent::MouseUp(k, _) if k == key))
}

#[test]
fn test_dt_timer_settlement_pops_waiting_stack() {
    let mut p = PipelineState::new(build_config());

    // 1) 按下 space：Waiting + 入栈
    p.key_down("space");
    assert_eq!(p.td("space").map(|s| s.kind), Some(TdKind::Waiting));
    assert!(on_stack(&p, "space"), "space 应在 waiting_stack（Waiting）");

    // 2) 抬起 space：TapDone + 调度 DoubleTap；此时仍须留栈供双击判定
    p.key_up("space");
    assert_eq!(p.td("space").map(|s| s.kind), Some(TdKind::TapDone));
    assert!(on_stack(&p, "space"), "TapDone 期间仍应留栈，否则双击解析错位");

    // 3) 双击窗口超时 → dt_timer 结算为单 tap
    p.advance_ms(260); // 必须跨过 deadline_tick=250（advance_ms 的循环条件是 tick < target）
    assert!(p.td("space").is_none(), "dt_timer 后 td 状态应被清掉");
    assert!(!on_stack(&p, "space"),
        "dt_timer 结算后 space 必须出栈；残留会让后续无 TD 键被错误 defer");
    assert!(emitted(&p, "space"), "dt_timer 应补发单 tap 的 Down(space)");
    assert!(emitted_up(&p, "space"), "dt_timer 应补发单 tap 的 Up(space)");
}

#[test]
fn test_after_dt_settlement_mouse_right_is_not_deferred() {
    let mut p = PipelineState::new(build_config());

    p.key_down("space");
    p.key_up("space");
    p.advance_ms(260); // 结算 TapDone（跨过 deadline_tick=250）
    assert!(!on_stack(&p, "space"), "前置条件：space 已出栈");

    // 右键在当前层无 TD 定义，栈空时应立即发 Down(mousemiddle)
    p.emit_log.clear();
    p.key_down("mouseright");
    assert!(!p.state.defer.keys_deferred.contains_key("mouseright"),
        "栈空时 mouseright 不应被延迟");
    assert!(emitted(&p, "mousemiddle"),
        "mouseright 按下时应立即发出 Down(mousemiddle)（可按住），修复前只会在抬起时补发");

    p.key_up("mouseright");
    assert!(emitted_up(&p, "mousemiddle"));
}

#[test]
fn test_td_interrupt_all_tapdone_also_pops_stack() {
    // 覆盖 flush_tapdone_as_tap 的另一个调用入口：第三键抬起时 td_interrupt_all
    // 打断处于 TapDone 的 space。
    let mut cfg = build_config();
    cfg.layers.layer_maps.entry("baseLayer".into()).or_default().insert("z".into(), KeyEntry {
        tap: "{z}".into(),
        hold: "{shift}".into(),
        ht: "200".into(),
        ..Default::default()
    });
    let mut p = PipelineState::new(cfg);

    p.key_down("space");
    p.key_down("z");
    assert!(on_stack(&p, "space"));

    p.key_up("space"); // space → TapDone
    assert_eq!(p.td("space").map(|s| s.kind), Some(TdKind::TapDone));

    p.key_up("z"); // td_interrupt_all 打断 space 的 TapDone
    assert!(!on_stack(&p, "space"),
        "被打断的 TapDone 键也必须出栈；修复前 force_hold_execute(z) 无法清掉它");
}

#[test]
fn test_double_tap_still_works_after_fix() {
    let mut p = PipelineState::new(build_config());

    p.key_down("space");
    p.key_up("space");   // TapDone + DoubleTap 计时器
    p.emit_log.clear();
    p.key_down("space"); // 第二次按下 → 双击确认

    assert_eq!(p.td("space").map(|s| s.kind), Some(TdKind::DoubleTapHold),
        "单击后 250ms 内的第二次按下仍应确认双击");
    assert!(emitted(&p, "f2"), "双击应发出配置的 dt 输出 Down(f2)");
    assert!(!on_stack(&p, "space"), "双击确认经 force_hold_execute 出栈（既有行为，不应被破坏）");

    p.key_up("space");
    assert!(emitted_up(&p, "f2"), "释放应发出 Up(f2)");
}
