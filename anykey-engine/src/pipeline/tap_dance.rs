// pipeline/tap_dance.rs — tapdance 状态机（tap_dance_down/up + timers + interrupt）

use crate::state::*;
use super::*;

impl PipelineState {

    /// flush_tapdone_as_tap: 把处于 TapDone 的键作为一次「完整 tap」补发（down + up）。
    /// TapDone 的物理键在第一次 keyup 时就已抬起，后续不会再有 keyup 事件，
    /// 所以任何「立即结算 TapDone」的路径都必须自己补发 Up，否则逻辑键卡在按下态（漏 UP）。
    /// 统一封装 `re_enter(Tap)` + `reenter_up`，供 dt_timer（定时器到期）、
    /// td_interrupt_all（被第三键打断）、phase_up2_flush（延迟键释放时源头键已 TapDone）
    /// 三条路径共用，避免各处手写时漏掉 reenter_up。
    fn flush_tapdone_as_tap(&mut self, key: &str) {
        // 去掉 re_enter：tap 的解析+发送统一交给 reenter_up（内部经 up4 解析 + up5 发送 + up6 清理），
        // 与真实 keyup 完全等价，避免重复路径。
        let mut ctx = Context::new(key);
        ctx.flags = REENTER_UP_SKIP;
        self.reenter_up(&mut ctx, REENTER_UP_SKIP);
    }

    /// _TapDanceDown: TD 按下状态机
    /// new → Waiting(启动 hold timer)
    /// Waiting → 拦截(auto-repeat)
    /// TapDone → DoubleWait(启动 doubleHold timer) 或 doubleTap
    /// Holding → holding(不改变)
    /// Finished → 重置
    pub fn tap_dance_down(&mut self, key: &str, ctx: &mut Context) -> bool {
        // holding / doubletap-hold → 跳过（auto-repeat 吸收，不再输出）
        if let Some(s) = self.state.keys.key_states.get(key).and_then(|__ks| __ks.td.as_ref()) {
            if s.kind == TdKind::Holding || s.kind == TdKind::DoubleTapHold {
                ctx.td_action = TdAction::Holding;
                return false;
            }
        }

        // finished → 标记 reset（同 AHK "reset"）
        if let Some(s) = self.state.keys.key_states.get_mut(key).and_then(|__ks| __ks.td.as_mut()) {
            if s.kind == TdKind::Finished {
                s.kind = TdKind::Reset;
            }
        }

        // 解析按键所处的锚点层
        let cur = match self.target_anchor_layer(key) {
            Some((_, l)) => l,
            None => {
                ctx.td_action = TdAction::Tap;
                return false;
            }
        };

        // 提取该层配置条目（键不存在可查的 cur 层时退化为纯 tap）
        // 提取后即刻将所有字段值捕获到局部变量中（避免 cfg 作为 self.keys 引用
        // 与后续 set_td_state 等 &mut self 调用冲突）。
        let (has_td, has_dh, has_dt) = match self.mapping.tap_dance.get(key).and_then(|m| m.get(&cur)) {
            Some(c) => (c.has_td(), !c.double_hold.is_empty(), !c.double_tap.is_empty()),
            None => {
                ctx.td_action = TdAction::Tap;
                return false;
            }
        };

        if !has_td {
            ctx.td_action = TdAction::Tap;
            self.set_td_state(key, TdKind::Finished);
            return false;
        }

        let is_switch = self.is_switch_key(key);
        let hold_term = self.key_hold_term(key, &cur);
        let _dt_term = self.key_dbl_tap_term(key, &cur);
        let dh_term = self.key_dbl_hold_term(key, &cur);

        // 已有 tdState（非 Reset）→ 处理第二次按下或 auto-repeat
        if let Some(s) = self.state.keys.key_states.get(key).and_then(|__ks| __ks.td.as_ref()) {
            if s.kind != TdKind::Reset {
                if s.kind == TdKind::TapDone {
                    self.cancel_timer(key, TimerKind::DoubleTap);
                    if has_dh {
                        self.set_td_state(key, TdKind::DoubleWait);
                        self.schedule_timer(TimerKind::DoubleHold, key, key, dh_term);
                        if self.is_double_switch_key(key) {
                            self.enter_waiting_stack_as(key, SwitchKind::DoubleHold);
                        }
                        return true;
                    }
                    if has_dt {
                        ctx.td_action = TdAction::DoubleTap;
                        self.set_td_state(key, TdKind::DoubleTapHold);
                        if ctx.force_hold_down.is_none() {
                            ctx.force_hold_down = Some(key.to_string());
                        }
                        return false;
                    }
                }
                return true;
            }
        }

        // 第一次按下（或 Reset 后）：有 TD 配置 → 进入 Waiting，启动 hold 定时器
        self.set_td_state(key, TdKind::Waiting);
        if is_switch { self.enter_waiting_stack_as(key, SwitchKind::Hold); }
        self.schedule_timer(TimerKind::Hold, key, key, hold_term);
        true
    }

    /// _TapDanceUp: TD 抬起状态机
    /// Waiting → tap 或 TapDone(启动 dt 定时器)
    /// Holding → hold-release
    /// DoubleWait → doubleTap 或 two-tap
    /// DoubleHolding → doubleHold-release
    pub fn tap_dance_up(&mut self, ctx: &mut Context) {
        let key = &ctx.key.clone();

        // 入口守卫：仅「有 TD 配置(hold/dt/dh)」或「本键曾被 defer（keyup 时仍在 keys_deferred）」的键进入状态机。
        // 与 tap_dance_down 的 `if !cfg.has_td()` 镜像：纯 tap / 无设置键（未被 defer）不在此处理，
        // 其 down 在 keydown（或 flush）已发，Up 由 phase_up6_release 经 emitted 统一发出，无需进 TD 状态机。
        // 曾 defer 的纯 tap 键必须进：其 keyup 负责 td_interrupt_all 打断/flush 源头（如 td_deferred_tapdone_flush），
        // 否则源头键只能等自身定时器结算，事件顺序错乱。
        let cur = self.target_anchor_layer(key).map(|(_, l)| l);
        let has_td = cur.as_ref().map_or(false, |l| {
            self.mapping.tap_dance.get(key).and_then(|m| m.get(l)).map_or(false, |cfg| cfg.has_td())
        });
        if !has_td {
            // 键有 td_state 残留（如 force-hold 留下的 Holding），但锚点层无 td 配置。
            // 放行到 match kind 分支处理状态转换，避免状态泄露。
            let has_td_state = self.state.keys.key_states.get(key)
                .and_then(|__ks| __ks.td.as_ref())
                .is_some();
            if !has_td_state {
                self.debug("TD", &format!("TapDanceUp key={} skipped (no td config, not deferred)", key));
                return;
            }
        }

        self.debug("TD", &format!("TapDanceUp key={} td_kind={:?}", key,
            self.state.keys.key_states.get(key).and_then(|__ks| __ks.td.as_ref()).map(|s| s.kind)));

        // ③-1: 无 TD 状态（combo 键 keyup 时 TD 未被 keydown 设为 Waiting，即 Option::None）。
        // 注：TdKind 无 None 变体；「无状态」指 key_states[key].td 为 Option::None。
        // 该 None 情形与 Some(TdKind::Waiting) 在下方合并为同一 match 分支（见 ③-0/③-1a）。
        let kind: Option<TdKind> = self.state.keys.key_states.get(key).and_then(|__ks| __ks.td.as_ref()).map(|s| s.kind);

        match kind {
            Some(TdKind::Holding) => {
                self.cancel_timer(key, TimerKind::Hold);
                ctx.td_action = TdAction::HoldRelease;
                self.set_td_state(key, TdKind::Finished);
                // 释放路径：Down 已由 hold_timer 经 phase8 发出，本 keyup 仅释放（key_up 已默认 skip up4/up5，flags 冗余但显式）
                ctx.flags |= F_UPCOMMIT;
            }
            // ③-0/③-1a: Waiting 与「无 TD 状态（Option::None，即 combo 键在 Seeking/SILENCE 下未进栈）」
            // 合并为同一路径。两者 tap / double-tap 结算逻辑完全一致，区别仅在于 Waiting 键在栈中、
            // None(combo)键不在栈中，而 remove_waiting_stack_item 本就不在任一分支内调用，故可安全合并。
            // [缺口修复] TD 键从 Waiting/None 抬起、即将发 tap 前，需 force-hold 其所在 waiting_stack 中
            // 【自己之前】的所有层键（不含自己），使 tap 在正确的激活层内解析（此前非层键 TD 键在栈中
            // 有待激活层键时抬起，tap 会在未激活层解析 → 某 BUG 根源）。统一改为「登记触发键」交
            // phase_up4_forcehold：up4 经 force_hold_execute 查询(before-self)+激活+移除自己。
            // 注意 force_hold_up 只在「无 dt/dh」的立即 tap 路径内登记：dt/dh 路径走 TapDone 由 dt_timer
            // 到期经 reenter_up 重新进入，届时由 TapDone/DoubleWait 分支自身登记 force_hold_up，
            // 避免本 keyup 周期提前激活层（键提前出栈导致后续 double-tap 解析错位）。
            Some(TdKind::Waiting) | None => {
                // [C-lookup] keyup 抬起先打断其它 waiting 键时，不再跳过 P6（keydown forcehold）：
                // 被 flush 的 waiting 键经 FLUSH_DEFERRED_REENTER 保留 P6，使其 tap 在正确的已激活层上下文解析。
                // 注：td_interrupt_all 已上提到 phase_up3_td（调用 tap_dance_up 之前）统一执行，此处不再调用。
                // [C-lookup] cur 采用分层查询（与 resolve_key_layer 统一口径）——
                // 优先本键在 waiting_stack 中的「上一层」层键所指层；本键不在栈则取栈顶；都没有回落当前层。
                // 使 dt/dh 检测与 dt_term 在正确层上下文计算（此前用 current_layer() 在层尚未激活时会漏检 dt）。
                let cur = self.target_anchor_layer(key).map(|(_, l)| l).unwrap_or_else(|| self.current_layer());
                let dt_term = self.key_dbl_tap_term(key, &cur);
                // 有 doubleTap/doubleHold → 启动 TapDone + dt 定时器（跳过解析，tap 延迟到 dt_timer 发）
                if let Some(td) = self.mapping.tap_dance.get(key).and_then(|m| m.get(&cur)) {
                    if !td.double_tap.is_empty() || !td.double_hold.is_empty() {
                        self.set_td_state(key, TdKind::TapDone);
                        self.cancel_timer(key, TimerKind::Hold);
                        self.schedule_timer(TimerKind::DoubleTap, key, key, dt_term);
                        // 排完计时器后跳过后续所有相位（up4/up5/up6）：
                        // tap 由 DoubleTap 到期经 reenter_up 统一结算，避免 keyup 当下与计时器双发。
                        ctx.flags |= F_UPCOMMIT | F_UPRELEASE;
                        return;
                    }
                }
                // [缺口修复] 仅「无 dt/dh」的立即 tap 路径登记 force_hold_up（dt/dh 路径由 TapDone 分支在计时器到期时登记）。
                ctx.force_hold_up = Some(key.clone());
                ctx.td_action = TdAction::Tap;
                // 现发 tap 输出：清 skip 标志，走 up4(解析)+up5(发送)
                ctx.flags &= !(F_UPCOMMIT);
                self.cancel_timer(key, TimerKind::Hold);
                self.cancel_timer(key, TimerKind::DoubleTap);
                self.debug("TD", &format!("TapDanceUp key={} => TAP", key));
                self.set_td_state(key, TdKind::Finished);
            }
            // ③-1b: Finished（纯 tap / 无设置键，keydown 已发 tap）→ keyup 拦截，不发任何输出
            Some(TdKind::Finished) => {
                ctx.td_action = TdAction::Intercept;
                // 放行 resolve（up4）以清空 logical_key，output（up5）自然无输出
                ctx.flags &= !(F_UPCOMMIT);
                self.debug("TD", &format!("TapDanceUp key={} => INTERCEPT (finished, no keyup output)", key));
            }
            // ③-2: 看到 TapDone → 与 DoubleWait 处理一样
            Some(TdKind::TapDone) => {
                // [U5] keyup 确认 doubletap：登记触发键交 up4 ForceHold，确保 doubletap 输出在已激活层上下文中解析。
                // 本键通常非 switch 键→force_hold_execute 全激活整栈 waiting；若本键恰为 switch 键→before-self 激活其前。
                ctx.force_hold_up = Some(key.clone());
                let cur = self.current_layer();
                if let Some(td) = self.mapping.tap_dance.get(key).and_then(|m| m.get(&cur)) {
                    if !td.double_tap.is_empty() {
                        ctx.td_action = TdAction::DoubleTap;
                    } else {
                        ctx.td_action = TdAction::TwoTap;
                    }
                }
                ctx.flags &= !(F_UPCOMMIT);
                self.set_td_state(key, TdKind::Finished);
            }
            Some(TdKind::DoubleWait) => {
                // [U5] keyup 确认 doubletap（doublehold 路径第二次抬起）：同 TapDone 登记 force_hold_up 交 up4 激活层。
                ctx.force_hold_up = Some(key.clone());
                let cur = self.current_layer();
                if let Some(td) = self.mapping.tap_dance.get(key).and_then(|m| m.get(&cur)) {
                    if !td.double_tap.is_empty() {
                        ctx.td_action = TdAction::DoubleTap;
                    } else {
                        ctx.td_action = TdAction::TwoTap;
                    }
                }
                // 现发 doubleTap/two-tap：清 skip 标志，走 up4(解析)+up5(发送)
                ctx.flags &= !(F_UPCOMMIT);
                self.set_td_state(key, TdKind::Finished);
            }
            Some(TdKind::DoubleHolding) => {
                ctx.td_action = TdAction::DoubleHoldRelease;
                self.set_td_state(key, TdKind::Finished);
                // 释放路径：Down 已由 dh_timer 经 phase8 发出，本 keyup 仅释放（key_up 已默认 skip up4/up5，flags 冗余但显式）
                ctx.flags |= F_UPCOMMIT;
            }
            Some(TdKind::DoubleTapHold) => {
                // doubletap 以「按住」方式发出，物理键松开时释放该输出。
                // 复用 HoldRelease：读 emitted（doubletap 输出），本 keyup 仅释放（key_up 已默认 skip up4/up5，flags 冗余但显式）。
                ctx.td_action = TdAction::HoldRelease;
                self.set_td_state(key, TdKind::Finished);
                ctx.flags |= F_UPCOMMIT;
            }
            Some(TdKind::Reset) => {}
        }

    }

    /// set_td_state: 更新 tdState 条目（调试记录旧→新状态）
    /// 只更新 td 子系统，保留 combo / emitted（旧架构中三张表独立，
    /// set_td_state 不应抹掉 keyDownMapping / comboState 的数据）
    pub(super) fn set_td_state(&mut self, key: &str, kind: TdKind) {
        let old = self.state.keys.key_states.get(key).and_then(|__ks| __ks.td.as_ref()).map(|s| format!("{:?}", s.kind));
        self.debug("TD", &format!("setState {}: {:?} -> {:?}", key, old, kind));
        let entry = self.state.keys.key_states.entry(key.to_string()).or_default();
        entry.td = Some(TdState { kind });
    }

    /// _TD_HoldTimer: hold_term 到期 → tdAction=hold → re_enter 回管道
    /// （P5 TD 状态机确认 + P7 commit 的 resolver 解析 hold、output 激活层/emit）
    pub fn hold_timer(&mut self, key: &str) {
        if !self.state.keys.key_states.get(key).map_or(false, |__ks| __ks.td.is_some()) { return; }
        if self.state.keys.key_states[key].td.as_ref().unwrap().kind != TdKind::Waiting { return; }

        // 1) ReEnter → 回 key_down 管道：resolver 解析 hold，output 激活层 / emit hold
        let mut ctx = Context::new(key);
        ctx.td_action = TdAction::Hold;
        self.re_enter(&mut ctx, COMMIT_SEND_REENTER);

        // 守卫: 重入期间状态可能被改变
        if !self.state.keys.key_states.get(key).map_or(false, |__ks| __ks.td.is_some()) || self.state.keys.key_states[key].td.as_ref().unwrap().kind != TdKind::Waiting {
            self.debug("TD", &format!("hold_timer key={} state changed during re-enter, skip", key));
            return;
        }

        // 2) 设置 Holding 状态（解析值由 phase7_commit 的 output 统一记入 emitted，无需在此暂存）
        //    注意：先不从 waiting_stack 移除 key——flush_deferred_entry 需要用 key 在栈中的
        //    位置切分「其之前(链式依赖)/之后(仍等待)」的边界，只激活之前的部分。
        self.cancel_timer(key, TimerKind::Hold);
        self.set_td_state(key, TdKind::Holding);

        // 3) Flush 该 TD 键关联的延迟键（此时 key 仍在栈中，供 flush 定位边界；
        //    key 自身已 Holding，force_hold 对它是幂等空操作，仅激活其栈下方链式依赖）
        let del_keys: Vec<String> = self.state.defer.keys_deferred.iter()
            .filter(|(_, v)| v.td_key == *key).map(|(k, _)| k.clone()).collect();
        for dk in del_keys { self.flush_deferred_entry(&dk); }

        // 4) key 转 hold 完成，从 waiting_stack 移除
        self.remove_waiting_stack_item(key);
    }

    /// _TD_DTTimer: double_tap_term 到期（TapDone 状态）→ 设 Finished +
    /// flush_tapdone_as_tap 结算为完整 tap（补发 down+up，逻辑键不会卡在按下态）
    pub fn dt_timer(&mut self, key: &str) {
        if !self.state.keys.key_states.get(key).map_or(false, |__ks| __ks.td.is_some()) { return; }
        if self.state.keys.key_states[key].td.as_ref().unwrap().kind != TdKind::TapDone { return; }

        // 定时器到期结算 TapDone：先设 Finished（让 phase_up8_clean 能清），
        // 再走统一 flush（含 reenter_up）清理输出残留。
        self.set_td_state(key, TdKind::Finished);
        self.flush_tapdone_as_tap(key);
    }

    /// _TD_DHTimer: double_hold_term 到期 → tdAction=doubleHold → 状态 DoubleHolding
    /// [C5.2] 计时器触发 doublehold 后需 forcehold：改用 flush_deferred_entry 同款通道
    /// （含 P6 ForceHold，skip_before(F_FORCEHOLD)），并以本键为触发键。这样：
    ///   - 普通 doublehold（C5.2）：其前 waiting switch 键被 before-self 激活，doublehold 输出在已激活层上下文解析；
    ///   - doublehold-switch（C5.1）：本键在栈(DoubleHold)，before-self 激活其前、移除自己，
    ///     doublehold（层键）经 P7 正常输出并激活层，出栈后不会重复 forcehold。
    pub fn dh_timer(&mut self, key: &str) {
        if !self.state.keys.key_states.get(key).map_or(false, |__ks| __ks.td.is_some()) { return; }
        if self.state.keys.key_states[key].td.as_ref().unwrap().kind != TdKind::DoubleWait { return; }

        self.set_td_state(key, TdKind::DoubleHolding);

        let mut ctx = Context::new(key);
        ctx.td_action = TdAction::DoubleHold;
        // 触发键=本键，交 P6 ForceHold 经 flush 通道激活 waiting stack（含自己为 DoubleHold 时 before-self）
        ctx.force_hold_down = Some(key.to_string());
        self.re_enter(&mut ctx, FLUSH_DEFERRED_REENTER);

        // 3) Flush 该 TD 键关联的延迟键（此时 key 仍在栈中，供 flush 定位边界；
        //    key 自身已 Holding，force_hold 对它是幂等空操作，仅激活其栈下方链式依赖）
        let del_keys: Vec<String> = self.state.defer.keys_deferred.iter()
            .filter(|(_, v)| v.td_key == *key).map(|(k, _)| k.clone()).collect();
        for dk in del_keys { self.flush_deferred_entry(&dk); }

        // doublehold 已确定，出栈（避免后续被重复 forcehold）；doublehold-switch 经 P6 已移除，此处 no-op。
        self.remove_waiting_stack_item(key);
        self.set_td_state(key, TdKind::DoubleHolding);
    }

    /// _TD_InterruptAll: 打断所有非层键的 TD waiting
    /// Waiting → tap | TapDone → tap | DoubleWait → doubleTap/two-tap
    /// `force_hold_before`：keydown 侧为 true（K4/C3b），被中断键退化 tap 前需 force-hold 其前
    ///   waiting switch 键；keyup 侧为 false（由 up4 的 force_hold_up 统一处理，避免重复 force）。
    pub fn td_interrupt_all(&mut self, key: &str, force_hold_before: bool) {
        let keys: Vec<String> = self.state.keys.key_states.keys().cloned().collect();
        for k in &keys {
            if k == key { continue; }
            if let Some(s) = self.state.keys.key_states.get(k).and_then(|__ks| __ks.td.as_ref()) {
                if s.kind == TdKind::Waiting && self.is_switch_key(k) {
                    continue; // 保护层键（hold=层/修饰）：交给 force_hold 通道激活，不在 td_interrupt_all 结算
                }
                if s.kind == TdKind::DoubleWait && self.is_double_switch_key(k) {
                    continue; // [C5.1] 保护 doublehold-switch 键：与 hold-switch 完全一致——
                               // 不在此主动结算，交给 force_hold 通道（后续键 before-self/fallback 或 dh_timer）激活 double_hold
                }
            }

            let kind = self.state.keys.key_states.get(k).and_then(|__ks| __ks.td.as_ref()).map(|s| s.kind);
            match kind {
                Some(TdKind::TapDone) => {
                    // TapDone 已被物理键抬起、后续无 keyup：打断 flush 发 down 后必须补发 up，
                    // 统一走 flush_tapdone_as_tap（含 reenter_up），否则逻辑键卡在按下态（漏 UP）。
                    self.flush_tapdone_as_tap(k);
                    self.set_td_state(k, TdKind::Finished);
                }
                Some(TdKind::Waiting) => {
                    // Waiting 键仍按住，真实 keyup 会稍后到来做释放，此处只结算 tap 输出。
                    // K4（C3b keydown）：非 switch 键 hold 被中断退化 tap 时，需在解析前 force-hold
                    // 其前 waiting switch 键，使 tap 在正确激活层解析。复用 K2(flush_deferred_entry) 通道：
                    // force_hold_down=k 经 FLUSH_DEFERRED_REENTER 保留 P6，k 非 stack 键→全激活其前 switch。
                    let mut re_ctx = Context::new(k);
                    re_ctx.td_action = TdAction::Tap;
                    if force_hold_before {
                        re_ctx.force_hold_down = Some(k.to_string());
                        self.re_enter(&mut re_ctx, FLUSH_DEFERRED_REENTER);
                    } else {
                        self.re_enter(&mut re_ctx, COMMIT_SEND_REENTER);
                    }
                    self.set_td_state(k, TdKind::Finished);
                }
                Some(TdKind::DoubleWait) => {
                    // 到达此分支的必为「普通 doublehold 键」——doublehold-switch 键已在上方被 skip，
                    // 其 double_hold 交由 force_hold 通道（后续键 before-self/fallback 或 dh_timer）激活。
                    // 第三键打断处于 DoubleWait 的普通 doublehold 键 → 退化成 doubletap/two-tap。
                    let tmp = self.resolve_key_output(k, "doubleTap");
                    let action = if !tmp.is_empty() { TdAction::DoubleTap } else { TdAction::TwoTap };
                    let mut re_ctx = Context::new(k);
                    re_ctx.td_action = action;
                    self.re_enter(&mut re_ctx, COMMIT_SEND_REENTER);
                    self.set_td_state(k, TdKind::Finished);
                }
                _ => continue,
            }
        }
    }

    /// is_double_switch_key: 检查逻辑键的 double_hold 值是否为层键 {fnX}/{bnX} 或修饰键。
    /// 与 is_switch_key 完全对称，但查 double_hold 字段（is_switch_key 查 hold 字段）。
    /// 用于 C5.1：doublehold-switch 键需在 DoubleWait 时入栈，发送时发 double_hold 而非 hold。
    /// 注意：不能把 is_switch_key 扩成也查 double_hold —— waiting_stack 既要记录又要查询发什么
    /// （见 defer 设计文档 C5.1），故此处独立判定、入栈时打 SwitchKind 标签区分。
    pub fn is_double_switch_key(&self, logical_key: &str) -> bool {
        if !self.mapping.tap_dance.contains_key(logical_key) { return false; }
        let layer = match self.target_anchor_layer(logical_key) {
            Some((_, l)) => l,
            None => return false,
        };
        let dh = match self.mapping.tap_dance[logical_key].get(&layer) {
            Some(e) => e.double_hold.clone(),
            None => return false,
        };
        if dh.is_empty() { return false; }
        if is_layer_key(&dh) { return true; }
        let kn = dh.trim_matches(|c| c == '{' || c == '}').to_lowercase();
        let mods = ["shift", "ctrl", "alt", "win", "lshift", "rshift", "lctrl", "rctrl", "lalt", "ralt", "lwin", "rwin"];
        mods.contains(&kn.as_str())
    }

}
