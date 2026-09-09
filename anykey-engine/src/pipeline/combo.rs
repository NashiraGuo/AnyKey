// pipeline/combo.rs — combo 系统（匹配 + 状态机 + interrupt + cleanup）

use crate::state::*;
use super::*;

impl PipelineState {

    /// CleanupReleased: 清理 comboState 中已 Released / Failed 的条目（phase1_cleanup 调用）
    pub(super) fn cleanup_released(&mut self) {
        let to_delete: Vec<String> = self.state.keys.key_states.iter()
            .filter(|(_, s)| {
                s.combo.as_ref().map_or(false, |c| {
                    c.state == ComboKind::Released
                        || c.state == ComboKind::Failed
                })
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in to_delete {
            self.clear_combo(&k);
            self.state.keys.pending_timers.remove(&k);
        }
    }


    /// 层感知查表 combo 输出：优先用匹配时记录的命中层（combo.layer），
    /// 其次 current_layer，最后回退 base（避免匹配后激活修饰键污染 current_layer 导致找不到输出）。
    pub(super) fn resolve_combo_output(&self, key: &str, partner: &str) -> String {
        if let Some(c) = self.combo(key) {
            let layer = &c.layer;
            if !layer.is_empty() {
                if let Some(o) = self.combo_lookup(key, partner, layer) { return o; }
            }
        }
        let layer = self.current_layer();
        if let Some(o) = self.combo_lookup(key, partner, &layer) { return o; }
        if layer != "base" {
            if let Some(o) = self.combo_lookup(key, partner, "base") { return o; }
        }
        String::new()
    }

    /// 三变量查表（当前键 + partner 键 + 层）：搜统一的 combo_map_index
    fn combo_lookup(&self, key: &str, partner: &str, layer: &str) -> Option<String> {
        if let Some(km) = self.mapping.combo.map.get(layer).and_then(|m| m.get(key)) {
            if let Some(o) = km.get(partner) { return Some(o.clone()); }
        }
        None
    }

    /// start_single_key_timeout: 为 combo 键启动单键超时定时器
    pub fn start_single_key_timeout(&mut self, key: &str) {
        let timeout = self.config.combo_time;
        self.debug("COMBO", &format!("StartSingleKeyTimeout key={} timeout={}", key, timeout));
        self.schedule_timer(TimerKind::ComboTimeout, key, key, timeout);
    }

    /// _RecordWaiting: 将键写入 comboState waiting 状态
    pub fn record_waiting(&mut self, key: &str) {
        if !self.has_combo(key) {
            let now = self.now();
            self.debug("COMBO", &format!("_RecordWaiting key={} now={}", key, now));
            self.state.keys.key_states.entry(key.to_string()).or_default().combo = Some(ComboState::waiting(key, now));
        }
    }

    /// try_match_combo: 在层 combo 映射中查找匹配的 waiting 伙伴键
    fn try_match_combo(&self, layer_map: &HashMap<String, HashMap<String, String>>, key: &str) -> (bool, String, String) {
        if !layer_map.contains_key(key) { return (false, String::new(), String::new()); }
        let now = self.now();
        for (o, out) in &layer_map[key] {
            if !self.state.keys.key_states.get(o).map_or(false, |__kc| __kc.combo.is_some()) { continue; }
            let pstate = self.state.keys.key_states[o].combo.as_ref().unwrap().state.clone();
            if pstate == ComboKind::Stuck { continue; }
            // 只匹配仍为 Waiting（尚未提交到其它 combo）的伙伴；已是 Holding/Active 的伙伴视为已占用，
            // 避免组合键重按时误重匹配同一伙伴（链式：b 已转入 b+c 时不应再触发 a+b）。
            if pstate != ComboKind::Seeking { continue; }
            let elapsed = now - self.state.keys.key_states[o].combo.as_ref().unwrap().down_time;
            if elapsed <= self.config.combo_time {
                return (true, o.clone(), out.clone());
            }
        }
        (false, String::new(), String::new())
    }

    /// _SetupComboState: 双向设置 combo 对的 partner/output/state
    pub(super) fn setup_combo_state(&mut self, key: &str, other: &str, output: &str, state_type: ComboKind) {
        if let Some(s) = self.state.keys.key_states.get_mut(key).and_then(|__kc| __kc.combo.as_mut()) {
            s.partner = other.to_string();
            s.output = output.to_string();
            s.state = state_type.clone();
        }
        if let Some(s) = self.state.keys.key_states.get_mut(other).and_then(|__kc| __kc.combo.as_mut()) {
            s.partner = key.to_string();
            s.output = output.to_string();
            s.state = state_type;
        }
    }

    /// _ComboInterruptAll: 第三键打断——把等待伙伴的 Seeking combo 键标记为 Interrupted
    /// （combo 未完成，重跑管道时 resolver 将其退化为本键 tap），移除 pending timer 并 re_enter。
    pub fn combo_interrupt_all(&mut self, key: &str) {
        let combo_keys: Vec<String> = self.state.keys.key_states.keys()
            .filter(|k| self.combo(k).map_or(false, |c| c.state == ComboKind::Seeking))
            .cloned().collect();
        for k in &combo_keys {
            if k == key { continue; }
            if let Some(s) = self.combo(k) {
                if s.state != ComboKind::Seeking { continue; }
            }
            if let Some(partners) = self.mapping.combo.partners.get(k) {
                if partners.contains(key) { continue; }
            }

            // 标记本键为「被打断」：combo 未完成（等伙伴中）被第三键打断，
            // 不再以 Seeking 重跑管道——改用 Interrupted 让 resolve_output 明确识别为「退化本键 tap」。
            if let Some(s) = self.state.keys.key_states.get_mut(k).and_then(|__kc| __kc.combo.as_mut()) {
                s.state = ComboKind::Interrupted;
            }
            let k2 = k.clone();
            self.state.keys.pending_timers.remove(&k2);
            let mut re_ctx = Context::new(&k2);
            self.re_enter(&mut re_ctx, COMBO_TIMEOUT_REENTER);
            // 不发 release_key：会立刻抬起正在按着的第一个 combo 键；
            // 去掉后可以继续按住，等待物理键抬起时经 keyup 发送并清理。
            // self.release_key(&k2);
            // self.clear_combo(&k2);
        }
    }

    /// _HandleComboRepeat: 按 comboState 决定自动重复行为
    /// Active→redirect | Holding→repeat | waiting→absorb | Stuck→absorb
    pub fn handle_combo_repeat(&mut self, ctx: &mut Context) {
        let key = &ctx.key;
        let state = match self.state.keys.key_states.get(key).and_then(|__kc| __kc.combo.as_ref()) {
            Some(s) => s.state.clone(),
            None => return,
        };
        match state {
            ComboKind::Active => { ctx.logical_key.clear(); }
            ComboKind::Holding => { ctx.flags |= COMBO_REPEAT_SKIP; }
            ComboKind::Seeking => { ctx.flags |= SILENCE; }
            ComboKind::Stuck => {}
            _ => return,
        }
        ctx.flags |= F_COMBO;  // 跳过 phase3
    }

    /// 统一匹配 combo（hold/macro 已合并为单表 combo_map_index）：
    /// 按 waiting_stack 层键目标层 → 活跃层栈 → base 顺序，返回 (partner, output)。
    /// 设计意图（按重构方案）：
    ///   - 不做 force_hold（层激活职责交给 phase4_defer(combo) / P6 ForceHold）；
    ///   - 不写 output（映射职责交给 phase7_commit 的层感知查表）。
    /// 输出是单键({X})还是多键({a}{b}{c}) 由 phase7_commit 的 is_single_key 决定 Holding/Active。
    pub(super) fn match_combo_unified(&self, key: &str) -> Option<(String, String, String)> {
        // 1) waiting_stack 层键目标层（栈顶到底）
        //    遍历等待栈，找每个 switch 键的目标层，尝试在该层匹配 combo。
        for (wk, _) in self.state.defer.waiting_stack.iter().rev() {
            if let Some(s) = self.state.keys.key_states.get(wk).and_then(|__ks| __ks.td.as_ref()) {
                if (s.kind == TdKind::Waiting || s.kind == TdKind::Holding) && self.is_switch_key(wk) {
                    let cur = self.current_layer();
                    if let Some(entry) = self.mapping.tap_dance.get(wk).and_then(|m| m.get(&cur)) {
                        if is_layer_key(&entry.hold) {
                            let layer = extract_layer_name(&entry.hold);
                            if let Some(r) = self.match_in_map(&self.mapping.combo.map, &layer, key) {
                                return Some((r.0, r.1, layer));
                            }
                        }
                    }
                }
            }
        }
        // 2) 活跃层栈（顶到底）
        for idx in (0..self.state.layers.layer_stack.len()).rev() {
            let layer = self.state.layers.layer_stack[idx].name.clone();
            if let Some(r) = self.match_in_map(&self.mapping.combo.map, &layer, key) { return Some((r.0, r.1, layer)); }
        }
        // 3) base 层
        if let Some(r) = self.match_in_map(&self.mapping.combo.map, "base", key) { return Some((r.0, r.1, "base".to_string())); }
        None
    }

    /// 在指定层的 combo map 中查找 key 的匹配伙伴（仅匹配仍为 Waiting 的伙伴）
    fn match_in_map(
        &self,
        map: &HashMap<String, HashMap<String, HashMap<String, String>>>,
        layer: &str,
        key: &str,
    ) -> Option<(String, String)> {
        let lm = map.get(layer)?;
        let (found, other, output) = self.try_match_combo(lm, key);
        if found { Some((other, output)) } else { None }
    }

    /// 判断 `key` 是否是其 combo 中「最后抬起」的键（即负责发 Up 的键）。
    ///
    /// 这里刻意【不维护显式的「是否最后抬起」标记字段】，而是用「伙伴键的当前状态」实时推导，
    /// 以避免状态字段更新时序不一致导致的混乱 bug（例如本键先标记 pending_up、伙伴后清理，
    /// 或链式 combo 中状态错位）。代价是这段逻辑不直观，故集中在此并用注释说明。
    ///
    /// 「伙伴已离场」等价于本键最后抬起，判定为以下三种情况之一：
    ///   1. 伙伴键的 combo 已不存在（已被清理）  → 伙伴已离场；
    ///   2. 伙伴键的 combo 状态 == Released      → 伙伴已抬起；
    ///   3. 伙伴键的 combo.output != 本键 output → 伙伴的 combo 槽已被另一个 combo 占用
    ///      （链式 combo：如 b 从 a+b 转入 b+c，则 a 的 a+b 视为已离场）。
    /// 三者都不满足 → 伙伴仍按住同一 output，本键非最后抬起（output 保持按下，本键静默）。
    pub fn combo_partner_departed(&self, key: &str) -> bool {
        let Some(c) = self.combo(key) else { return false; };
        let partner = c.partner.clone();
        let output = c.output.clone();
        self.combo(&partner)
            .map_or(true, |p| p.state == ComboKind::Released || p.output != output)
    }

    /// _ResolveKeyUp: KeyUp combo 释放（仅写状态，不发送按键）。
    /// 在 phase_up1_combo 调用，遵循「combo 阶段只写状态、发送放 final」分层：
    ///   - 标记本键 ComboState.state = Released；
    ///   - 若本键是「最后抬起」，置 pending_up=true（标记本键为 Up 发送者），
    ///     实际发送推迟到 phase_up6_release 的 release_key 用 combo.output 完成。
    ///   - emitted/target_layer 不参与 combo 释放决策（在 release_key 发送后随本键一并清理）。
    ///
    /// 判定「是否最后抬起」：伙伴已 Released，或伙伴的 combo 槽已被不同 output 占用
    /// （链式：b 已转入 b+c，a 的 a+b 视为已离场）→ 本键负责发 Up（置 pending_up）；
    /// 非最后抬起（伙伴仍在按住同一 output）：output 保持按下，本键静默。
    pub fn resolve_key_up(&mut self, ctx: &mut Context, state_obj: &ComboState) -> bool {
        let key = ctx.key.clone();
        match state_obj.state {
            ComboKind::Holding => {
                // 是否「最后抬起」：见 combo_partner_departed 的判定说明
                let partner_released = self.combo_partner_departed(&key);

                if partner_released {
                    // 本键是「最后抬起」→ 标记本键为 combo Up 的发送者（pending_up=true），
                    // 实际发送推迟到 phase_up6_release 的 release_key 用 combo.output 完成
                    // （与「phase_up2 只写状态、发送放 final」的分层一致）。
                }
                // 非最后抬起（伙伴仍在按住）→ output 保持按下，本键静默；
                // emitted 由 phase_up6_release 末尾按本键自身 release 清理，不在此清伙伴。
                // 释放路径：跳过 phase_up3_td（combo 活跃/释放期间成员键不进 TD 状态机）、
                // phase_up5_commit 与 phase_up6_output（显式 flag 控制，取代隐晦内部判断）
                ctx.flags |= F_UPTAPDANCE | F_UPCOMMIT;
                // 仅标记 Released + 标记 Up 发送者，不清理（phase_up6_release 末尾收尾）
                if let Some(s) = self.state.keys.key_states.get_mut(&key).and_then(|__kc| __kc.combo.as_mut()) {
                    s.state = ComboKind::Released;
                    if partner_released { s.pending_up = true; }
                }
                true
            }
            ComboKind::Active => {
                // 宏 combo 无 Up，仅标记 Released，不清理
                // 释放路径：跳过 phase_up3_td（combo 成员键不进 TD 状态机）、
                // phase_up5_commit 与 phase_up6_output
                ctx.flags |= F_UPTAPDANCE | F_UPCOMMIT;
                if let Some(s) = self.state.keys.key_states.get_mut(&key).and_then(|__kc| __kc.combo.as_mut()) {
                    s.state = ComboKind::Released;
                }
                true
            }
            _ => false,
        }
    }

    /// single_key_timeout: Combo 超时 — deferred→remove+reEnter | waiting→Stuck
    pub fn single_key_timeout(&mut self, key: &str) {
        if !self.state.keys.key_states.get(key).map_or(false, |__kc| __kc.combo.is_some()) { return; }
        let kind = self.state.keys.key_states[key].combo.as_ref().unwrap().state.clone();
        // [DEADCODE-PROBE] 探测 1418 分支是否可达：combo 超时键是否同时落在 keys_deferred 里
        self.debug("DEADCODE", &format!("TimeoutProbe key={} state={:?} inKeysDeferred={}", key, kind, self.state.defer.keys_deferred.contains_key(key)));
        if kind != ComboKind::Seeking {
            self.clear_combo(key);
            self.state.keys.pending_timers.remove(key);
            return;
        }

        if self.state.defer.keys_deferred.contains_key(key) {
            // [DEADCODE-PROBE] 若此分支被命中，说明该"死代码"实际可达，需重新审视
            self.debug("DEADCODE", ">>> BRANCH HIT: single_key_timeout found key in keys_deferred (REACHABLE!)");
            // 已确认死代码（2026-07-08 探针测试证实不可达）：
            // combo-Waiting 键在 phase3 会被 SILENCE(含 F_DEFER) 跳过 phase4_defer，
            // 故 keys_deferred 中永远不会包含 combo 超时键；此处分支恒为 false。
            // —— 2026-07-08 起临时注释掉执行体，保留探测器，待日常使用测试确认后再删除。
            //    若未来真走到这里，会自然落到下方 Stuck 安全路径（不再单独 clear+release）。
            // self.clear_combo(key);
            // self.state.keys.pending_timers.remove(key);
            // self.defer.keys_deferred.remove(key);
            // let mut re_ctx = Context::new(key);
            // self.re_enter(&mut re_ctx, DEFERRED_KEY_REENTER);
            // self.release_key(key);
            // return;
        }

        if let Some(s) = self.state.keys.key_states.get_mut(key).and_then(|__kc| __kc.combo.as_mut()) { s.state = ComboKind::Stuck; }
        self.state.keys.pending_timers.remove(key);
        self.state.defer.keys_deferred.remove(key);
        // self.pending_modifier_keys.remove(key);  // dead code — removed
        let mut re_ctx = Context::new(key);
        self.re_enter(&mut re_ctx, COMBO_TIMEOUT_REENTER);
    }

}