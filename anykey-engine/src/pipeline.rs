
/// AnyKey engine — Pipeline 主干（调度 + phase wrappers + 共享基础层）
/// 各系统实现拆分至子模块：combo.rs / defer.rs / tap_dance.rs / leader.rs / commit.rs
///
/// KeyDown:  Phase0_interrupt → 1_cleanup → 2_repeat → 3_combo → 4_defer → 5_tap_dance → 6_forcehold → 7_commit
/// KeyUp:    Up1_combo → Up2_flush → Up3_td → Up4_forcehold → Up5_commit → Up6_release → Up7_flushbehind → Up8_clean → UpLeader
///
/// Re-enter: 体外循环通过 re_enter 回到 KeyDown，skipFlags 控制跳过哪些 Phase

use std::collections::HashMap;
use std::cmp::Reverse;
use crate::state::*;
use crate::util::*;

mod combo;
mod defer;
mod tap_dance;
mod leader;
mod commit;

impl PipelineState {
    // ═══════════════════════════════════════════
    // Helpers / 关联常量
    // ═══════════════════════════════════════════

    /// 返回 layerStack 栈顶层名，空栈返回 "base"
    pub fn current_layer(&self) -> String {
        self.state.layers.layer_stack.last().map(|e| e.name.clone()).unwrap_or_else(|| "base".to_string())
    }

    /// is_switch_key: 检查逻辑键的 hold 值是否为层键 {fnX}/{bnX} 或修饰键
    /// 经 target_anchor_layer 取 anchor 层（最近 Waiting 层键目标层 → 当前层）→ 只读该层字段，读不到即非 switch（严格链式）
    pub fn is_switch_key(&self, logical_key: &str) -> bool {
        if !self.mapping.tap_dance.contains_key(logical_key) { return false; }
        // 与 target_anchor_layer 统一口径：取 anchor 层（最近 Waiting 层键目标层 → 当前层）→ 只读该层字段，读不到即非 switch（严格链式）
        let layer = match self.target_anchor_layer(logical_key) {
            Some((_, l)) => l,
            None => return false,
        };
        let hold = match self.mapping.tap_dance[logical_key].get(&layer) {
            Some(e) => e.hold.clone(),
            None => return false,
        };
        if hold.is_empty() { return false; }
        if is_layer_key(&hold) { return true; }
        let kn = hold.trim_matches(|c| c == '{' || c == '}').to_lowercase();
        let mods = ["shift", "ctrl", "alt", "win", "lshift", "rshift", "lctrl", "rctrl", "lalt", "ralt", "lwin", "rwin"];
        mods.contains(&kn.as_str())
    }


    /// 解析键的「anchor 层」——仅决定 TD 定义应查哪一层，不检查 key 是否在该层有定义。
    /// 严格链式：消费方（is_switch_key / is_double_switch_key / defer 锚点判定）在返回的 anchor 层
    /// 读字段，读不到即视为「该上下文下无此性质」。算法（2026-07-12，用户指定）：
    ///   1. 定位 key 在 waiting_stack 的位置 → pos；不在栈中则 i=len（等价从栈顶向下扫）。
    ///   2. 从 i-1 向下查最近的 Waiting 层键 → 其目标层即 anchor，到此结束。
    ///   3. 整段无层键 → anchor = current_layer()（自身回落 base）。
    /// target_anchor_layer: 返回 (anchor_key, anchor_layer)
    /// anchor_key 是 waiting_stack 中决定当前键上下文的最接近层键名（空串表示无层键）。
    /// anchor_layer 是 anchor_key 的目标层名（若无层键则回落 current_layer）。
    fn target_anchor_layer(&self, key: &str) -> Option<(String, String)> {
        if !self.mapping.tap_dance.contains_key(key) { return None; }

        // 分支 A：key 在 waiting_stack 中（pos=i）。从 pos-1 向下找最近 Waiting 层键，
        // 其目标层即 anchor（只认最近一个，不查 key 在该层有无定义）。栈中间键用自己上层的层键上下文，不被栈顶误判。
        if let Some(pos) = self.state.defer.waiting_stack.iter().position(|(w, _)| w == key) {
            let mut i = pos;
            while i > 0 {
                i -= 1;
                let wk = &self.state.defer.waiting_stack[i].0;
                if let Some(td_s) = self.state.keys.key_states.get(wk).and_then(|__ks| __ks.td.as_ref()) {
                    if td_s.kind == TdKind::Waiting {
                        let wk_cur = self.current_layer();
                        if let Some(entry) = self.mapping.tap_dance.get(wk).and_then(|m| m.get(&wk_cur)) {
                            if is_layer_key(&entry.hold) {
                                let pl = extract_layer_name(&entry.hold);
                                return Some((wk.clone(), pl));
                            }
                        }
                    }
                }
            }
            return Some((String::new(), self.current_layer()));
        }

        // 分支 B：key 不在 waiting_stack 中（i=len，从栈顶向下扫）。从栈顶找最近 Waiting 层键，
        // 其目标层即 anchor（只认最近一个，不查 key 在该层有无定义）；整段无层键则用当前层为 anchor。
        for (wk, _) in self.state.defer.waiting_stack.iter().rev() {
            if wk.as_str() == key { continue; }
            if let Some(td_s) = self.state.keys.key_states.get(wk).and_then(|__ks| __ks.td.as_ref()) {
                if td_s.kind == TdKind::Waiting {
                    let wk_cur = self.current_layer();
                    if let Some(entry) = self.mapping.tap_dance.get(wk).and_then(|m| m.get(&wk_cur)) {
                        if is_layer_key(&entry.hold) {
                            let pl = extract_layer_name(&entry.hold);
                            return Some((wk.clone(), pl));
                        }
                    }
                }
            }
        }
        Some((String::new(), self.current_layer()))
    }

    /// _ResolveKeyOutput: 按层栈解析键输出
    /// tap 遍历层栈（base 兜底），hold/dt/dh 仅查当前层（回退 base）
    pub fn resolve_key_output(&self, td_key: &str, action: &str) -> String {
        if !self.mapping.tap_dance.contains_key(td_key) { return String::new(); }
        let k = &self.mapping.tap_dance[td_key];

        if action == "tap" {
            for idx in (0..self.state.layers.layer_stack.len()).rev() {
                if let Some(entry) = k.get(&self.state.layers.layer_stack[idx].name) {
                    if !entry.tap.is_empty() { return entry.tap.clone(); }
                }
            }
            if let Some(entry) = k.get("base") {
                if !entry.tap.is_empty() { return entry.tap.clone(); }
            }
            return String::new();
        }

        let cur = self.current_layer();
        // hold / doubleTap / doubleHold：同一字段映射逻辑，依次查当前层、base 层（避免重复 match）
        for layer in [cur.as_str(), "base"] {
            if let Some(entry) = k.get(layer) {
                let val = match action {
                    "hold" => &entry.hold,
                    "doubleTap" => &entry.double_tap,
                    "doubleHold" => &entry.double_hold,
                    _ => return String::new(),
                };
                if !val.is_empty() { return val.clone(); }
            }
        }
        String::new()
    }

    // ── 层激活/管道重入 ──


    /// re_enter: 管道重入 — skip 传语义化跳过集（如 SEND_REENTER），标记已完成阶段，回到 key_down_inner
    pub fn re_enter(&mut self, ctx: &mut Context, skip: u32) {
        self.debug("PIPE", &format!("KDown: {} flags=0x{:02X} [re-enter]", ctx.key, ctx.flags | skip));
        ctx.flags |= skip;
        self.key_down_inner(ctx);
    }

    // ═══════════════════════════════════════════
    // KeyDown: Phase 0-7
    // ═══════════════════════════════════════════

    // --- Phase0: 碰撞打断 -- combo_interrupt_all ---
    fn phase0_interrupt(&mut self, ctx: &mut Context) {
        if ctx.flags & F_INTERRUPT != 0 {
            self.debug("PIPE", &format!("[SKIP] PHASE0-Interrupt key={}", ctx.key));
            return;
        }
        self.debug("PIPE", &format!("PHASE0-Interrupt key={}", ctx.key));
        ctx.flags |= F_INTERRUPT;
        let key = ctx.key.clone();
        self.combo_interrupt_all(&key);
        // TD 模块入口（phase0 打断 phase）：keydown 侧 force_hold_before=true
        self.td_interrupt_all(&key, true);
    }

    // --- Phase1: 状态清理 -- cleanup_released ---
    fn phase1_cleanup(&mut self, ctx: &mut Context) {
        if ctx.flags & F_CLEANUP != 0 {
            self.debug("PIPE", &format!("[SKIP] PHASE1-Cleanup key={}", ctx.key));
            return;
        }
        self.debug("PIPE", &format!("PHASE1-Cleanup key={}", ctx.key));
        ctx.flags |= F_CLEANUP;
        self.cleanup_released();
    }

    // --- Phase2: 长按重复 -- handle_combo_repeat ---
    fn phase2_repeat(&mut self, ctx: &mut Context) {
        if ctx.flags & F_REPEAT != 0 {
            self.debug("PIPE", &format!("[SKIP] PHASE2-Repeat key={} logicalKey={} flags=0x{:02X}", ctx.key, ctx.logical_key, ctx.flags));
            return;
        }
        self.debug("PIPE", &format!("PHASE2-Repeat key={} logicalKey={} flags=0x{:02X}", ctx.key, ctx.logical_key, ctx.flags));
        self.handle_combo_repeat(ctx);
        ctx.flags |= F_REPEAT;
    }

    // --- Phase3: Combo 匹配 -- hold/macro combo / 单键超时 ---
    fn phase3_combo(&mut self, ctx: &mut Context) {
        if ctx.flags & F_COMBO != 0 {
            self.debug("PIPE", &format!("[SKIP] PHASE3-Combo key={}", ctx.key));
            return;
        }
        ctx.flags |= F_COMBO;
        let key = &ctx.key.clone();
        let is_combo = self.mapping.combo.key_set.contains(key);
        let td_count = self.state.keys.key_states.len();
        self.debug("COMBO", &format!("PHASE3 key={} isCombo={} layerStack={} tdCount={}", key, is_combo, self.state.layers.layer_stack.len(), td_count));
        let is_combo = self.mapping.combo.key_set.contains(key);

        if is_combo {
            self.record_waiting(key);
        }

        // 统一匹配 hold/macro combo（同时搜两表）。确定性强 → 跳过 defer(P5)。
        // 仅标记 Matched + 记录命中层：不写 output / 不写 logical_key / 不激活层；
        // 映射交给 phase7_commit（层感知查表），层激活交给 phase4_defer(combo) / P6 ForceHold。
        if let Some((other, output, layer)) = self.match_combo_unified(key) {
            self.debug("COMBO", &format!("PHASE3-MATCH key={} other={} output={} layer={}", key, other, output, layer));
            self.setup_combo_state(key, &other, "", ComboKind::Matched);
            // 记录命中层，供 phase7_commit 层感知查表（避免匹配后激活修饰键污染 current_layer）
            if let Some(s) = self.state.keys.key_states.get_mut(key).and_then(|ks| ks.combo.as_mut()) { s.layer = layer.clone(); }
            if let Some(s) = self.state.keys.key_states.get_mut(&other).and_then(|ks| ks.combo.as_mut()) { s.layer = layer.clone(); }
            self.cancel_timers_for_key(key);
            self.cancel_timers_for_key(&other);
            ctx.flags |= F_DEFER;
            // combo 命中：写入 force_hold_down 哨兵，交 P6 ForceHold 在 phase7_commit 查表前全激活整栈 waiting
            // （需完整层上下文，非 before-key 规则；修复「层 combo 匹配后未激活 waiting_stack 修饰键 → 输出顺序颠倒」bug）。
            // 放在此处（而非 phase4_defer）以紧贴 combo 匹配成功点：本键随即被标记 F_DEFER 跳过延迟决策，
            // 但 force-hold 激活仍须在 phase7 查表前完成。
            ctx.force_hold_down = Some(crate::state::FORCE_HOLD_COMBO_ALL.to_string());
            ctx.flags |= F_TAPDANCE; // combo 59cb7ec88df38fc7 TD 96366bb5
            // 注意：此处【不能】设置 F_COMMIT。映射已移交给 phase7_commit，
            // 若这里跳过，phase7 就永远不会对 Matched 的 combo 做 (当前键+partner+层) 查表，
            // 导致 combo 永远停在 Matched 状态、发不出正确的 combo 输出（只发原始键）。
            return;
        }

        // 单键超时（无伙伴）
        if is_combo {
            self.start_single_key_timeout(key);
            ctx.flags |= SILENCE;
            // [DEADCODE-PROBE] combo-Waiting 键被 SILENCE（含 F_DEFER），phase4_defer 被跳过 → 不会进入 keys_deferred
            self.debug("DEADCODE", &format!("Phase3-SILENCE key={} -> phase4_defer skipped", key));
        }
    }

    // --- Phase4: Defer -- flush deferred 键 + 延迟决策；不主动 force-hold、不插手 TD 中断 ---
    // combo 命中激活已上移到 phase3_combo（匹配成功即写 ctx.force_hold_down 哨兵，交 P6 ForceHold 全激活整栈）；
    // flush deferred 写入 re_ctx.force_hold_down（触发键=被 flush 延迟键），统一由 P6 ForceHold 经重播激活。
    fn phase4_defer(&mut self, ctx: &mut Context) {
        if ctx.flags & F_DEFER != 0 {
            self.debug("PIPE", &format!("[SKIP] PHASE4-DEFER key={} wsLen={} tdCount={} defCount={}", ctx.key, self.state.defer.waiting_stack.len(), self.state.keys.key_states.len(), self.state.defer.keys_deferred.len()));
            return;
        }
        self.debug("PIPE", &format!("PHASE4-DEFER key={} wsLen={} tdCount={} defCount={}", ctx.key, self.state.defer.waiting_stack.len(), self.state.keys.key_states.len(), self.state.defer.keys_deferred.len()));
        ctx.flags |= F_DEFER;

        // 1) 发送所有 defer 键（防止堆积）；flush_deferred_entry 不再主动 force-hold，
        //    改为把触发键(被 flush 的延迟键)写入 re_ctx.force_hold_down，由 P6 ForceHold 经重播完成激活。
        let deferred_keys: Vec<String> = self.state.defer.keys_deferred.keys().cloned().collect();
        for pk in deferred_keys {
            self.flush_deferred_entry(&pk);
        }

        // 2) 新延迟判定
        if self.try_defer_layer_interrupt(ctx) && !self.state.keys.key_states.is_empty() {
            ctx.flags |= SILENCE;
            return;
        }
    }

    // --- Phase5: TD 决策 -- tap_dance_down 状态机 ---
    fn phase5_tap_dance(&mut self, ctx: &mut Context) {
        // 所有按键都进 TD 状态机。守卫和安全阀全部在 tap_dance_down 内部处理。
        if ctx.flags & F_TAPDANCE != 0 {
            self.debug("PIPE", &format!("[SKIP] PHASE5-TD key={}", ctx.key));
            return;
        }
        self.debug("PIPE", &format!("PHASE5-TD key={} tdState={}", ctx.key,
            self.state.keys.key_states.get(&ctx.key).and_then(|__ks| __ks.td.as_ref()).map_or("none".to_string(), |s| format!("{:?}", s.kind))));

        let key = ctx.key.clone();
        if self.tap_dance_down(&key, ctx) {
            ctx.clear_combo = true;
            ctx.td_pending = self.state.keys.key_states.get(&ctx.key).and_then(|__ks| __ks.td.as_ref()).is_some();
        }
    }

    // --- Phase6: ForceHold -- 消费 ctx.force_hold_down 触发键，激活 waiting switch 键（置于 resolver 前）---
    //   包装层：阶段头 flag 检测 + 入口标志 + 调用统一 force_hold_execute（trigger=force_hold_down）。
    fn phase6_forcehold(&mut self, ctx: &mut Context) {
        if ctx.flags & F_FORCEHOLD != 0 {
            self.debug("PIPE", &format!("[SKIP] PHASE5-ForceHold key={}", ctx.key));
            return;
        }
        self.debug("PIPE", &format!("PHASE5-ForceHold key={}", ctx.key));
        ctx.flags |= F_FORCEHOLD;                     // 入口 ctx 检测标志（兼标记本阶段完成）
        if let Some(trigger) = ctx.force_hold_down.take() {
            self.force_hold_execute(ctx, &trigger);
        }
    }

    // --- Phase7: commit_stage — resolver + output（合并原 P7 resolve + P8 final）---
    //    wrapper，含 td_pending 守卫。resolve 与 output 之间预留了新 Phase 插入点（见 commit_stage）。
    fn phase7_commit(&mut self, ctx: &mut Context) {
        if ctx.flags & F_COMMIT != 0 {
            self.debug("PIPE", &format!("[SKIP] PHASE7-Resolve key={}", ctx.key));
            return;
        }
        self.debug("PIPE", &format!("PHASE7-Resolve key={} logicalKey={} tdAction={:?} flags=0x{:02X}", ctx.key, ctx.logical_key, ctx.td_action, ctx.flags));
        if ctx.td_pending { return; }

        self.commit_stage(ctx);
        ctx.flags |= F_COMMIT;
    }




    // ═══════════════════════════════════════════
    // KeyUp: Up1 → Up8 + UpLeader
    // ═══════════════════════════════════════════
    // 注：函数定义顺序与执行顺序不同（key_up_inner 按 Up1→Up8 执行，
    // 源码中 phase_up2_flush 定义在 phase_up1_combo 之前、phase_up7_flushbehind 在 phase_up6_release 之前）。

    // --- Up1: Combo KeyUp — waiting→ForceHold+ReEnter | STUCK→clean | 其他→resolve_key_up ---
    fn phase_up1_combo(&mut self, ctx: &mut Context) {
        if ctx.flags & F_UPCOMBO != 0 {
            self.debug("PIPE", &format!("[SKIP] Up-Combo key={}", ctx.key));
            return;
        }
        self.debug("PIPE", &format!("Up-Combo key={} inComboState={}", ctx.key, self.state.keys.key_states.get(&ctx.key).map_or(false, |__kc| __kc.combo.is_some())));
        let key = &ctx.key.clone();
        if !self.state.keys.key_states.get(key).map_or(false, |__kc| __kc.combo.is_some()) { return; }
        let state = self.state.keys.key_states[key].combo.as_ref().unwrap().clone();

        match state.state {
            // Seeking：combo 未完成（伙伴未触发）被提前抬起。本阶段只关计时器 + 路由 resolver：
            // - cancel_timer(ComboTimeout)：关掉 single-key timeout（仅 pending_timers.remove 不够，需懒删标记）
            // - cancel_timer(Hold)：停掉 keydown 起的 Hold 计时器，避免延迟发 hold
            // - 清 up4/up5 的 skip，路由到 resolver→output→release（combo 回退为本键 tap；dt/dh 键由
            //   tap_dance_up 的 Waiting/None 分支处理成 TapDone 后重新 skip）
            // 不清 combo / 不清 TD（TD 保持 Waiting/None 交 tap_dance_up；combo 回收交 phase_up6_release）。
            ComboKind::Seeking => {
                // combo 未完成(伙伴未触发)被提前抬起：登记触发键交 phase_up4_forcehold 在 up4 激活
                // waiting stack 中【自己之前】的所有层键，确保回退为本键 tap 时在正确的层上下文解析。
                // 不出栈（combo Seeking 不在此移除 waiting_stack，保持旧语义）。
                if let Some(c) = self.state.keys.key_states.get_mut(key).and_then(|__kc| __kc.combo.as_mut()) {
                    c.state = ComboKind::Failed;
                }
                ctx.force_hold_up = Some(key.clone());
                self.cancel_timer(key, TimerKind::ComboTimeout);
                self.cancel_timer(key, TimerKind::Hold);
                ctx.flags &= !(F_UPCOMMIT);
            }
            // Stuck：单键超时已 re_enter 发过 tap；本阶段不写状态，交 phase_up6_release 清理 combo / timer。
            // （兼容旧行为：keyup 默认 skip up4/up5，up6 释放 timeout 已发的 tap。）
            ComboKind::Stuck => {}
            // Holding/Active/Released：保持「只写状态」——交给 resolve_key_up 标 Released + pending_up，
            // 发送由 release_key（phase_up6）完成；本阶段不清理。
            _ => {
                self.debug("UP", &format!("Up-Combo key={} state={:?}", key, state.state));
                self.resolve_key_up(ctx, &state);
                ctx.logical_key.clear();
            }
        }
    }

    // --- Up2: 处理延迟条目残留 — waiting/doubleWait→forceHolding ---
    fn phase_up2_flush(&mut self, ctx: &mut Context) {
        if ctx.flags & F_UPFLUSH != 0 {
            self.debug("PIPE", &format!("[SKIP] Up-Pending key={}", ctx.key));
            return;
        }
        self.debug("PIPE", &format!("Up-Pending key={}", ctx.key));
        let key = &ctx.key.clone();
        if let Some(info) = self.state.defer.keys_deferred.remove(key) {
            if let Some(s) = self.state.keys.key_states.get(&info.td_key).and_then(|__ks| __ks.td.as_ref()) {
                match s.kind {
                    TdKind::Waiting => {
                        // 延迟源头键(等待判定)抬起：登记触发键交 phase_up4_forcehold 在 up4 统一激活。
                        // 当前键=被抬起的延迟源键(非层键故不在栈) → up4 计算时走「全激活」分支，与旧「全栈 Waiting」一致。
                        // 不出栈（延迟源键本就不在 waiting_stack，且非 TD Waiting 结算路径）。
                        ctx.force_hold_up = Some(ctx.key.clone());
                    }
                    TdKind::DoubleWait => {
                        // 延迟源头键(双按确认期)抬起：同样登记触发键交 up4 force-hold。
                        // 此时 td_key 是 doublehold 层键(DoubleWait 已确认)，用 force_hold_up 在 up4
                        // 按 DoubleHold 标签激活其层，与 Waiting 分支对称。
                        ctx.force_hold_up = Some(ctx.key.clone());
                    }
                    _ => {}
                }
            }
            // 延迟键的 tap 现发输出：清 skip 标志，走 up4(解析)+up5(发送)
            ctx.flags &= !(F_UPCOMMIT);
        }
    }

    // --- Up3: TD 抬起 — tap_dance_up 状态机 ---
    fn phase_up3_td(&mut self, ctx: &mut Context) {
        if ctx.flags & F_UPTAPDANCE != 0 {
            self.debug("PIPE", &format!("[SKIP] Up-TD key={}", ctx.key));
            return;
        }
        let key = ctx.key.clone();
        self.debug("PIPE", &format!("Up-TD key={} inTD={}", key, self.state.keys.key_states.get(&key).map_or(false, |__ks| __ks.td.is_some())));
    self.debug("UP", &format!("Up-TD key={}", key));
    // 改动1：td_interrupt_all 从 tap_dance_up 内部提到 phase 层（phase_up3_td），
    // 所有 keyup 键（含纯 tap）在进 TD 状态机前都会先打断/flush 源头键。
    self.td_interrupt_all(&key, true);
    self.tap_dance_up(ctx);
    }

    // --- Up4: ForceHold — 对称 keydown P6，置于 tapdance(up3) 后、resolver(up5) 前 ---
    //   包装层：结构同 keydown phase6_forcehold，用 keyup 体系标志 F_UPFORCEHOLD（与 keydown 的 F_FORCEHOLD 对称、位域独立）。
    //   消费【keyup 专用字段】ctx.force_hold_up（与 keydown 的 force_hold_down 严格区分，杜绝跨侧串扰）。
    //   keyup 侧分支(phase_up2_flush 延迟源头键 Waiting / phase_up1_combo Seeking / tap_dance_up Waiting)
    //   写入触发键，统一在此处经 force_hold_execute 激活+移除自己，彻底收敛到这条对称通道。
    fn phase_up4_forcehold(&mut self, ctx: &mut Context) {
        if ctx.flags & F_UPFORCEHOLD != 0 {
            self.debug("PIPE", &format!("[SKIP] Up-ForceHold key={}", ctx.key));
            return;
        }
        self.debug("PIPE", &format!("Up-ForceHold key={}", ctx.key));
        ctx.flags |= F_UPFORCEHOLD;                     // 入口 ctx 检测标志（兼标记本阶段完成）
        if let Some(trigger) = ctx.force_hold_up.take() {
            self.force_hold_execute(ctx, &trigger);
        }
    }

    // --- Up5: commit_stage — resolver + output（对称 keydown P7）---
    fn phase_up5_commit(&mut self, ctx: &mut Context) {
        if ctx.flags & F_UPCOMMIT != 0 {
            self.debug("PIPE", &format!("[SKIP] Up-Commit key={}", ctx.key));
            return;
        }
        self.debug("PIPE", &format!("Up-Commit key={} tdAction={:?}", ctx.key, ctx.td_action));
        self.commit_stage(ctx);
        ctx.flags |= F_UPCOMMIT;
    }

    // --- Up6: 仅发送 — release_key 读 emitted/target_layer/combo 发 Up 或反激活层（清理拆到 phase_up7_clean）---
    fn phase_up6_release(&mut self, ctx: &mut Context) {
        if ctx.flags & F_UPRELEASE != 0 {
            self.debug("PIPE", &format!("[SKIP] Up-Release key={}", ctx.key));
            return;
        }
        self.debug("PIPE", &format!("Up-Release key={} logicalKey={} emitted={:?} targetLayer={:?}",
            ctx.key, ctx.logical_key, self.emitted(&ctx.key), self.target_layer(&ctx.key)));

        // 统一 release（读 emitted 处理层反激活 / 单键 Up）。本阶段只发，不清状态——
        // 状态清理交给 phase_up7_clean，使 flushbehind（位于 release 与 clean 之间）能在 K 状态清除前 flush 背后延迟键。
        let key = ctx.key.clone();
        self.release_key(&key);
    }

    // --- Up7: flush-behind ---
    //   语义：K 若为某等待层键(anchor)，其抬起后背后被延迟的键 n(keys_deferred[n].td_key == K)
    //   应随之结算发出（经 flush_deferred_entry 重入 P4~P8）。仅当 K 自身 TD 已 Finished 才触发，
    //   纯释放路径(非 TD 键)K.td=None → 不触发，等价于 no-op。
    fn phase_up7_flushbehind(&mut self, ctx: &mut Context) {
        if ctx.flags & F_UPFLUSHBEHIND != 0 {
            self.debug("PIPE", &format!("[SKIP] Up-FlushBehind key={}", ctx.key));
            return;
        }
        ctx.flags |= F_UPFLUSHBEHIND;
        let k_kind = self.td(&ctx.key).map(|s| s.kind);
        self.debug("PIPE", &format!("Up-FlushBehind key={} tdKind={:?}", ctx.key, k_kind));

        // 仅当当前键 K 的 TD 状态为 Finished 才 flush 其背后延迟键（K 交互已完成，背后键可结算）。
        if k_kind != Some(TdKind::Finished) { return; }

        // 收集 keys_deferred 中与 K 绑定的延迟键 n（DeferInfo.td_key == K），按名稳定排序后逐个 flush。
        // 先快照再 flush：flush_deferred_entry 内部 remove(n) 并重入，避免迭代中修改 HashMap。
        let mut behind: Vec<String> = self.state.defer.keys_deferred.iter()
            .filter(|(_, info)| info.td_key == ctx.key)
            .map(|(n, _)| n.clone())
            .collect();
        behind.sort();
        for n in behind {
            self.debug("PIPE", &format!("Up-FlushBehind flush deferred n={} behind K={}", n, ctx.key));
            self.flush_deferred_entry(&n);
        }
    }

    // --- Up8: 清理 — 清本键 emitted/target_layer 残留 + td + combo（含伙伴）；发送已由 phase_up6_release 完成 ---
    //   置于 flushbehind 之后：flushbehind 在 K 状态仍在时 flush 背后延迟键 n（emit Down n + 记 emitted），
    //   本阶段再把 K 的 emitted/td/combo 清掉，不影响 n（n 的清理在其自身 keyup 走同一 clean 路径）。
    fn phase_up8_clean(&mut self, ctx: &mut Context) {
        if ctx.flags & F_UPCLEAN != 0 {
            self.debug("PIPE", &format!("[SKIP] Up-Clean key={}", ctx.key));
            return;
        }
        ctx.flags |= F_UPCLEAN;
        self.debug("PIPE", &format!("Up-Clean key={} emitted={:?} targetLayer={:?}",
            ctx.key, self.emitted(&ctx.key), self.target_layer(&ctx.key)));

        let key = ctx.key.clone();
        // —— 本键输出残留清理（与 release_key 的「发送」解耦：up7 发 Up/反激活层，up9 清状态）——
        // 无条件清理本键 emitted / target_layer：幂等，对 combo / td / 普通键均安全
        // （combo 成员键不写入这两个字段，clear 是 no-op；普通键/层键的清理此前散落在 release_key，现统一在此）。
        self.clear_emitted(&key);
        self.clear_target_layer(&key);
        self.clear_leader_blk(&key);   // Leader 拦截标记：本键抬起即清（与 emitted/target_layer 同等幂等清理，无需 (a) 阶段单独处理）

        if let Some(s) = self.state.keys.key_states.get(&ctx.key).and_then(|__ks| __ks.td.as_ref()) {
            if s.kind == TdKind::Finished {
                self.clear_td(&ctx.key);
            }
        }

        // combo 收尾（最后一步）：按 combo.state 回收 + 伙伴输出残留一并清掉。
        // - Released：本键与伙伴都已 Released 才清（resolve_key_up 只写状态、不清理，
        //   确保 release_key 读 combo 状态判断 partner 是否已抬起时 combo 仍在）。
        // - Failed（combo 未完成被提前抬起，由 phase_up1 的 Seeking 转入）/ Stuck（单键超时）/ Interrupted：
        //   combo 匹配失败路径，清理本键与伙伴 combo + 移除 pending timer。
        //   （force-hold waiting switch 键已在 phase_up1_combo(Waiting) 完成，此处不再重复。）
        if let Some(c) = self.combo(&ctx.key).cloned() {
            match c.state {
                ComboKind::Released => {
                    let partner_released = self.combo(&c.partner)
                        .map_or(true, |p| p.state == ComboKind::Released);
                    if partner_released {
                        // combo 不再使用 emitted/target_layer：与 release_key 的发送互斥后，
                        // 在此一并清掉本键 + 伙伴的输出残留与 combo 簿记。
                        self.clear_combo(&ctx.key);
                        self.clear_emitted(&c.partner);
                        self.clear_target_layer(&c.partner);
                        self.clear_combo(&c.partner);
                    }
                }
                ComboKind::Failed | ComboKind::Stuck | ComboKind::Interrupted => {
                    self.clear_combo(&ctx.key);
                    if !c.partner.is_empty() {
                        self.clear_combo(&c.partner);
                        self.clear_emitted(&c.partner);
                        self.clear_target_layer(&c.partner);
                    }
                    self.state.keys.pending_timers.remove(&ctx.key);
                    self.state.keys.pending_timers.remove(&c.partner);
                }
                _ => {}
            }
        }
    }

    /// reenter_up (keyup 重入): 等价于一次 keyup 清理路径。物理键已抬起、后续不会有 keyup，
    /// 用于计时器输出（dt_timer 的 tap、SleepTimer 延迟后缀）。
    /// 调用前设好 ctx.flags（推荐 REENTER_UP_SKIP = 跳过 up1~up3+up7），
    /// 跑 up4→up6+up8 做纯清理。清除 td 由调用方负责：
    /// dt_timer 提前设 Finished 让 phase_up8_clean 清；SleepTimer 无 td 为 no-op。
    pub fn reenter_up(&mut self, ctx: &mut Context, skip: u32) {
        self.debug("PIPE", &format!("KUp: {} flags=0x{:02X} [re-enter]", ctx.key, ctx.flags | skip));
        ctx.flags |= skip;
        self.key_up_inner(ctx);
    }


    /// fire_sleep_timer: SleepTimer 触发 —— 延迟后缀输出。
    /// 后缀已带有完整 logical_key，直接调 run_resolve_output 经 emit_output 发 Down（记 emitted），
    /// 再以 reenter_up 同 key 走清理（发 Up + 清 emitted/td/combo），与真实 keyup 等价。
    pub fn fire_sleep_timer(&mut self, entry: &TimerEntry) {
        let mut ctx = Context::new(&entry.key);
        ctx.logical_key = entry.extra_data.clone();
        // 后缀已有 logical_key，SEND_REENTER 会跳过 P7 导致 output 也不跑。
        // 改用 COMMIT_SEND_REENTER（skip P0-P6），P7 中 resolve 因 logical_key 非空自动跳过→run output。
        ctx.flags = COMMIT_SEND_REENTER;
        self.re_enter(&mut ctx, COMMIT_SEND_REENTER);
        // 后缀输出结束，走 reenter_up 清理。
        // Context::new(key) 会把 logical_key 设为 {key}，但此处无输出要发（后缀已由 re_enter 发出），
        // 必须清空 logical_key，否则 output 会把 {sleepMacro} 当键名发送。
        let mut up_ctx = Context::new(&entry.key);
        up_ctx.logical_key.clear();
        up_ctx.flags = REENTER_UP_SKIP;
        self.reenter_up(&mut up_ctx, REENTER_UP_SKIP);
    }

    // ═══════════════════════════════════════════
    // KeyDown / KeyUp 公开入口
    // ═══════════════════════════════════════════

    /// KeyDown: 创建 Context，调用 key_down_inner (Phase0-7)
    pub fn key_down(&mut self, key: &str) {
        self.debug("INPUT", &format!("DN {}", key));
        // 持久化触发设备 ID：管道内 emit 全从这里读，不依赖 self.current_device
        self.state.keys.key_states.entry(key.to_string()).or_default().device_id = self.current_device;
        let mut ctx = Context::new(key);
        self.key_down_inner(&mut ctx);
    }

    /// KeyDown: 管道入口 — 按序执行 Phase0→Phase7
    pub fn key_down_inner(&mut self, ctx: &mut Context) {
        self.debug("INPUT", &format!("KDown: {} flags=0x{:02X}", ctx.key, ctx.flags));
        self.phase0_interrupt(ctx);
        self.phase1_cleanup(ctx);
        self.phase2_repeat(ctx);
        self.phase3_combo(ctx);
        self.phase4_defer(ctx);          // P4: flush deferred + 延迟决策（TD 中断已上移 P5）
        self.phase5_tap_dance(ctx);     // P5: TD 模块入口（TD 中断统一由 phase0_interrupt 执行）
        self.phase6_forcehold(ctx);      // P6: 消费 force_hold_down 触发键，激活 waiting switch 键
        self.phase7_commit(ctx);        // P7: commit_stage（resolver + output）
    }

    /// KeyUp: 创建 Context，调用 key_up_inner
    pub fn key_up(&mut self, key: &str) {
        self.debug("INPUT", &format!("UP {}", key));
        self.debug("PIPE", &format!("KUp: {}", key));
        // keyup 默认走「纯释放」路径：跳过 up5_commit（resolver+output），
        // 直接到 up6_release。需要「现发新输出」的分支（tap/双敲/延迟键）
        // 在对应分支清掉 F_UPCOMMIT（见 phase_up2_flush / tap_dance_up）。
        let mut ctx = Context {
            flags: F_UPCOMMIT,
            ..Context::new(key)
        };
        self.key_up_inner(&mut ctx);
    }

    /// KeyUp 管道 — 顺序执行 Up1→Up8（含 phase_up_leader）
    pub fn key_up_inner(&mut self, ctx: &mut Context) {
        self.phase_up1_combo(ctx);
        self.phase_up2_flush(ctx);
        self.phase_up3_td(ctx);
        self.phase_up4_forcehold(ctx);
        self.phase_up5_commit(ctx);
        self.phase_up6_release(ctx);
        self.phase_up7_flushbehind(ctx);
        self.phase_up8_clean(ctx);
        self.phase_up_leader(ctx);      // Leader（Step4）：统一收进 keyup 管道末尾，key_up / reenter_up 共用
    }
}




