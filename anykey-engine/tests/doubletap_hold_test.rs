/// AnyKey engine — 双击（无 doublehold）按住流程回归测试
/// 复现用户日志场景：键 "1" 同时有 hold + double_tap（无 doublehold），
/// 双击第二次按下后【不松开一直按住】，验证：
///   1) doubletap 以「按住」方式发出（Down/Up，而非旧版 EmitText 的 down+up 文本）
///   2) 首次抬起启动的 DoubleTap 计时器在第二次按下确认时被取消，不会重复发送
///   3) 按住期间 Hold 计时器不会重新触发（DoubleTapHold 吸收 auto-repeat），不会退化成 hold 输出
///   4) 物理键释放时只发一次 Up（无双发）
///   5) 按住期间插入的【其他键】事件必须位于 Down(3) 与 Up(3) 之间 —— 以此证明
///      Up(3) 是「物理键 1 抬起」触发的，而非第二次 key_down 时立即 down+up（EmitText 行为）。

use anykey_engine::config::*;
use anykey_engine::state::*;
use serde_json::json;

fn build_config() -> Config {
    let cfg_json = json!({
        "comboTime": 200,
        "tapDance": {
            "baseLayer": {
                "1": { "tap": "{1}", "hold": "{1}", "dt": "{3}" },
                "2": { "tap": "{2}" }
            },
            "holdTerm": 200,
            "doubleTapTerm": 150,
            "doubleHoldTerm": 150
        },
    });
    serde_json::from_value(cfg_json).unwrap()
}

fn log_strings(p: &PipelineState) -> Vec<String> {
    p.emit_log.iter().map(|e| match e {
        EmitEvent::Down(k, _) => format!("Down({})", k),
        EmitEvent::Up(k, _) => format!("Up({})", k),
        EmitEvent::Tap(k, _) => format!("Tap({})", k),
        EmitEvent::Text(t, _) => format!("Text({})", t),
        EmitEvent::Run(c, _) => format!("Run({})", c),
        EmitEvent::LayerOn(l, _) => format!("LayerOn({})", l),
        EmitEvent::LayerOff(l, _) => format!("LayerOff({})", l),
        _ => format!("{:?}", e),
    }).collect()
}

#[test]
fn test_doubletap_no_doublehold_held() {
    let mut p = PipelineState::new(build_config());
    p.debug_enabled = true;

    // 第一次 tap（TD 设计：首拍被缓冲抑制，phase8 因 td_pending 提前返回，不立即发出）
    p.key_down("1");
    p.key_up("1");
    // 第二次 tap 并【按住】
    p.key_down("1");
    // 确认双击后 DoubleTap 计时器必须被取消（用户日志中的 bug：未取消会重复触发）
    assert!(
        !p.has_active_timer("1", TimerKind::DoubleTap),
        "第二次按下确认后 DoubleTap 计时器应被取消"
    );
    // 按住期间推进时间，超过 hold_term 与 double_tap_term：
    // DoubleTap 计时器已取消；Hold 计时器不应被重新调度（DoubleTapHold 吸收 auto-repeat）
    p.advance_ms(400);
    // 模拟若干 auto-repeat（物理键仍按住）
    p.key_down("1");
    p.key_down("1");
    p.key_down("1");

    // ★ 关键：按住期间插入【另一个键 2】的按下+抬起。
    //   若 doubletap 是 EmitText（down+up 立即发），则 Up(3) 会在本次之前，
    //   序列会变成 Down(3) Up(3) Down(2) Up(2)。
    //   正确行为：Up(3) 只能在物理键 1 真正抬起时发生，因此 Down(2)/Up(2) 必须夹在
    //   Down(3) 与 Up(3) 之间。
    p.key_down("2");
    p.key_up("2");

    // 最终释放物理键 1 → 才发 Up(3)
    p.key_up("1");

    let strings = log_strings(&p);

    // 关键不变量：doubletap 不再以 Text(down+up) 发出，也没有空 Text 噪声
    assert!(
        !strings.iter().any(|s| s.starts_with("Text(")),
        "doubletap 不应再以 Text 形式发出，且 auto-repeat 不应产生空 Text: {:?}",
        strings
    );

    // 完整期望序列：双击按住 → Down(3)，期间其他键 → Down(2)/Up(2)，释放 → Up(3)。
    // 顺序本身即证明 Up(3) 绑定于物理键 1 的抬起。
    assert_eq!(
        strings,
        vec!["Down(3)", "Down(2)", "Up(2)", "Up(3)"],
        "emit_log 不符（重点看 Up(3) 是否位于其他键事件之后）: {:?}",
        strings
    );

    // 释放后 TD 状态应已清理（finished）
    assert!(p.td("1").is_none(), "释放后 td(1) 应已清理");
}

#[test]
fn test_doubletap_no_doublehold_doubletap_timer_cancelled() {
    // 验证第二次按下确认后，DoubleTap 计时器被取消，advance 不会再触发 dt_timer
    let mut p = PipelineState::new(build_config());

    p.key_down("1");
    p.key_up("1");           // TapDone + 调度 DoubleTap(150ms)
    p.key_down("1");         // 确认双击 → 取消 DoubleTap，DoubleTapHold
    // 若 DoubleTap 计时器未被取消，advance 会触发 dt_timer 重复发送，使 emit_log 多出事件
    p.advance_ms(300);

    let strings = log_strings(&p);
    // 期望：仅一次双击按下 Down(3)；没有额外的 Text/Tap/Up（dt_timer 不复发，Hold 不触发）
    let extra: Vec<&String> = strings.iter()
        .filter(|s| s.starts_with("Text(") || s.starts_with("Tap(") || s.starts_with("Up("))
        .collect();
    assert!(
        extra.is_empty(),
        "DoubleTap 计时器未被正确取消，产生了多余事件: {:?}",
        strings
    );
    assert_eq!(strings, vec!["Down(3)"], "emit_log 不符: {:?}", strings);
}

fn build_config_hold_only() -> Config {
    // 仅 hold 字段的 TD 键：tap 缺省为键自身 {1}
    let cfg_json = json!({
        "comboTime": 200,
        "tapDance": {
            "baseLayer": {
                "1": { "hold": "{2}" }
            },
            "holdTerm": 200,
            "doubleTapTerm": 150,
            "doubleHoldTerm": 150
        },
    });
    serde_json::from_value(cfg_json).unwrap()
}

#[test]
fn test_hold_only_doubletap_no_premature_hold() {
    // 复现用户日志 bug：仅 hold 字段的 TD 键，快速双击应得 "11" 而非 "12"。
    // 关键：第一次 tap 抬起时 Hold 计时器被取消；第二次按下重新调度 Hold 时，
    // 不能「复活」旧的（已取消）Hold 计时器导致提前触发 hold 输出 {2}。
    let mut p = PipelineState::new(build_config_hold_only());
    p.debug_enabled = true;

    p.key_down("1");
    p.key_up("1");                       // 第一次 tap：Down(1)/Up(1)，调度并取消 Hold(A, deadline=200)
    assert_eq!(log_strings(&p), vec!["Down(1)", "Up(1)"]);

    p.advance_ms(170);                   // tick=170，逼近 Hold(A) 截止
    p.key_down("1");                     // 第二次按下：调度 Hold(B, deadline=370)
    p.advance_ms(60);                    // tick=230：Hold(A) 到期，Hold(B) 尚未到期

    // ★ 修复前：schedule_timer 清掉了 (1,Hold) 取消标记 → 旧 Hold(A) 被「复活」提前触发 → Down(2)
    assert!(
        !log_strings(&p).iter().any(|s| s.contains('2')),
        "第二次按下后、Hold(B) 到期前，旧 Hold 计时器不应提前触发: {:?}",
        log_strings(&p)
    );

    p.key_up("1");                       // 第二次 tap 抬起：Down(1)/Up(1)，取消 Hold(B)

    // 双击应得 "11"，而非 hold 触发的 "12"
    assert_eq!(
        log_strings(&p),
        vec!["Down(1)", "Up(1)", "Down(1)", "Up(1)"],
        "双击应得 \"11\"，而非 hold 提前触发的 \"12\": {:?}",
        log_strings(&p)
    );
}

fn build_config_dt_no_hold() -> Config {
    // 仅 tap + dt（double_tap，无 hold），用于触发 dt_timer 且避免 hold_timer 干扰。
    // 注意：引擎配置键是 "dt"（非 "double_tap"），与 build_config 中 "dt":"3" 一致。
    let cfg_json = json!({
        "comboTime": 200,
        "tapDance": {
            "baseLayer": {
                "1": { "tap": "{a}", "dt": "{aa}" }
            },
            "holdTerm": 200,
            "doubleTapTerm": 150,
            "doubleHoldTerm": 150
        },
    });
    serde_json::from_value(cfg_json).unwrap()
}

#[test]
fn test_dt_timer_fires_and_cleans_via_reenter_up() {
    // dt_timer 在物理键已抬起（TapDone）后触发，发出单 tap 输出，
    // 并通过 reenter_up（等价于真实 keyup 的 phase_up6_release）做清理：
    // 发 Up + 清 emitted/td。验证「脱手后发送」的计时器输出与真实 keyup 等价、无残留。
    let mut p = PipelineState::new(build_config_dt_no_hold());
    p.key_down("1");
    p.key_up("1");                       // TapDone + 调度 DoubleTap(150ms)，无即时输出（tap 被缓冲）
    assert!(log_strings(&p).is_empty(), "TapDone 阶段不应立即发输出: {:?}", log_strings(&p));
    assert!(p.td("1").is_some(), "应处于 TapDone（td 仍存在）");

    p.advance_ms(200);                   // 触发 dt_timer → 发出单 tap 并经 reenter_up 清理

    let strings = log_strings(&p);
    // dt_timer 经 re_enter(phase8) 发 Down、再经 reenter_up 发 Up，形成完整 Down+Up 自包含输出；
    // 重点验证 reenter_up 做了等价于真实 keyup 的「发 Up + 清状态」，不漏 Up、不卡键。
    // 锁死修复：配置 tap={a}，单拍超时后必须发配置的 tap 而非键自身 {1}。
    assert_eq!(
        strings,
        vec!["Down(a)", "Up(a)"],
        "dt_timer 应发出配置的 tap {{a}}（修复前会发键自身 Down(1)/Up(1)）: {:?}",
        strings
    );
    // 清理等价真实 keyup：td 与 emitted 均不残留
    assert!(p.td("1").is_none(), "dt_timer 经 reenter_up 应清掉 td 状态");
    assert!(p.emitted("1").is_none(), "dt_timer 经 reenter_up 应清掉 emitted");
}

#[test]
fn test_sleep_timer_suffix_emitted_and_cleaned() {
    // SleepTimer 后缀（延迟输出）经 re_enter 发出并记 emitted，再由 reenter_up 清理：
    //   - 前缀立即发，后缀延迟发
    //   - 延迟后发出后缀
    //   - 发出后合成键 {sleepMacro} 的 emitted / key_states 不残留（不污染真实键 "1"）
    // 直接调用 send_key 触发 {Sleep} 分段逻辑（与真实 tap 输出路径一致），隔离 SleepTimer 行为。
    let cfg = json!({
        "comboTime": 200,
        "tapDance": { "baseLayer": { "1": { "tap": "{a}" } } },
    });
    let mut p = PipelineState::new(serde_json::from_value(cfg).unwrap());
    let mut ctx = Context::new("1");
    ctx.logical_key = "Pre{Sleep 50}Post".to_string();
    p.send_key(&mut ctx);   // 前缀 Pre 立即发；后缀 Post 延迟 50ms

    // 延迟前：前缀已发、后缀未发
    let before = log_strings(&p);
    assert!(before.iter().any(|s| s == "Text(Pre)"), "前缀应立即发出: {:?}", before);
    assert!(
        !before.iter().any(|s| s.contains("post")),
        "延迟前不应发出后缀: {:?}",
        before
    );

    p.advance_ms(60);                    // 越过 50ms 延迟 → SleepTimer 触发
    let strings = log_strings(&p);
    assert!(
        strings.iter().any(|s| s == "Text(Post)"),
        "延迟后应发出后缀 Post: {:?}",
        strings
    );

    // 清理：合成物理键 {sleepMacro} 不应残留 emitted
    assert!(
        p.emitted("{sleepMacro}").is_none(),
        "reenter_up 应清掉合成键 sleepMacro 的 emitted"
    );
    // 真实键释放：宏前缀 Pre 是 fire-and-forget 文本，记 emitted 后由 release 清除（不发 Up）
    p.key_up("1");
    // 真实键 "1" 未被 sleep 后缀污染
    assert!(
        p.emitted("1").is_none(),
        "真实键 1 不应被 sleep 后缀污染"
    );
}

#[test]
fn test_waiting_tap_uses_emit_down() {
    // 验证 Waiting 抬起单拍（tap="{a}" ≠ 键自身）现在走 emit_down + 记 emitted + release 发 Up，
    // 而非 emit_tap 自包含。期望：精确 Down(a)/Up(a)，无双发；release 后 emitted 清空。
    let cfg = json!({
        "comboTime": 200,
        "tapDance": { "baseLayer": { "1": { "tap": "{a}" } } },
    });
    let mut p = PipelineState::new(serde_json::from_value(cfg).unwrap());

    p.key_down("1");
    // 抬起时仍按住（未到 holdTerm）→ Waiting→tap 单拍
    p.key_up("1");

    let strings = log_strings(&p);
    assert_eq!(strings, vec!["Down(a)", "Up(a)"],
        "Waiting 单拍应 emit_down+release 发 Down(a)/Up(a)，无双发: {:?}", strings);
    // release 后 emitted 应已清空（不残留到下次按下）
    assert!(p.emitted("1").is_none(), "release 后 emitted(1) 应清空: {:?}", p.emitted("1"));
}
