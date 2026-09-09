// pipeline/defer.rs — defer 系统（延迟决策 + force-hold + waiting_stack）

use crate::state::*;
use super::*;

impl PipelineState {

    /// 统一 ForceHold 执行：接收「触发键」trigger，激活其【之前】(不含自己)的 waiting switch 键，再移除触发键自己。
    /// 等待栈只含 switch 键，且 force_hold_switch_key 激活即出栈，故执行后栈中仅剩触发键【之后】的键继续等待。
    ///
    /// trigger 三类语义：
    ///   - == FORCE_HOLD_COMBO_ALL（combo 命中）：全激活整栈（需完整层上下文，非 before-key 规则）。
    ///   - 普通键且在栈中：激活其之前(waiting_stack[..idx])，再移除自己。
    ///   - 普通键不在栈中（被 flush 的延迟键 / 非层键 flush 源）：回退全激活整栈。
    /// 任一「全激活」后整栈清空，再 remove(trigger) 为 no-op；combo 哨兵/延迟键本非真键，remove 亦 no-op
    /// ——「激活前面」与「移除自己」两动作在所有分支均安全，无需额外分支判断。
    pub(super) fn force_hold_execute(&mut self, ctx: &Context, trigger: &str) {
        self.debug("FH", &format!("ForceHoldExecute key={} trigger={}", ctx.key, trigger));
        let to_activate: Vec<(String, SwitchKind)> = if trigger == crate::state::FORCE_HOLD_COMBO_ALL {
            self.state.defer.waiting_stack.clone()
        } else {
            match self.state.defer.waiting_stack.iter().position(|(k, _)| k == trigger) {
                Some(idx) => self.state.defer.waiting_stack[..idx].to_vec(),
                None => self.state.defer.waiting_stack.clone(),
            }
        };
        for (k, kind) in &to_activate {
            // 按入栈标签发对应字段：Hold→hold（层/修饰键），DoubleHold→double_hold（见 C5.1）
            self.force_hold_switch_key(k, *kind, "_ForceHold");
        }
        // 移除触发键自己：找到时它不在子集内（子集不含自己），需手动 remove；
        // 没找到时整栈已清空（全激活），remove 是 no-op。sentinel/延迟键非真键，remove 亦 no-op。
        self.remove_waiting_stack_item(trigger);
    }

    /// remove_waiting_stack_item: 从 waitingStack 中移除指定键
    pub fn remove_waiting_stack_item(&mut self, key: &str) {
        self.state.defer.waiting_stack.retain(|(k, _)| k != key);
    }

    /// enter_waiting_stack_as: 将键以指定 SwitchKind 标签压入 waiting_stack。
    /// 已存在则升级/保持标签：Hold→DoubleHold 升级（双按确认后改为按 doublehold 解读），
    /// DoubleHold 不被降级（已是更强语义）；同标签保持。用于 C5.1：doublehold-switch 键
    /// 在 DoubleWait 时由 Hold（若 hold 也是层键）升级或新压为 DoubleHold。
    pub fn enter_waiting_stack_as(&mut self, key: &str, kind: SwitchKind) {
        if let Some(pos) = self.state.defer.waiting_stack.iter().position(|(k, _)| k == key) {
            let new_kind = match (self.state.defer.waiting_stack[pos].1, kind) {
                (SwitchKind::DoubleHold, _) => SwitchKind::DoubleHold,
                (SwitchKind::Hold, SwitchKind::DoubleHold) => SwitchKind::DoubleHold,
                (SwitchKind::Hold, SwitchKind::Hold) => SwitchKind::Hold,
            };
            self.state.defer.waiting_stack[pos].1 = new_kind;
        } else {
            self.state.defer.waiting_stack.push((key.to_string(), kind));
        }
    }

    /// _ForceHoldSwitchKey: 将 TD waiting 键强制激活，触发 fire_switch。
    /// 按入栈标签（SwitchKind）决定发什么、置什么状态：
    ///   - Hold：要求 TD 状态为 Waiting，发 hold 字段（层/修饰键），置 Holding；
    ///   - DoubleHold（C5.1）：要求 TD 状态为 DoubleWait，发 double_hold 字段，置 DoubleHolding。
    /// 调用方（force_hold_execute）传入的键均来自 waiting_stack 或全激活整栈（均带标签），
    /// 故原 is_switch_key 守卫冗余已删；保留 TD 状态 / 输出字段守卫防止重复激活。
    pub fn force_hold_switch_key(&mut self, td_key: &str, kind: SwitchKind, _caller: &str) -> bool {
        if !self.state.keys.key_states.get(td_key).map_or(false, |__ks| __ks.td.is_some()) { return false; }
        let expect = match kind {
            SwitchKind::Hold => TdKind::Waiting,
            SwitchKind::DoubleHold => TdKind::DoubleWait,
        };
        if self.state.keys.key_states[td_key].td.as_ref().unwrap().kind != expect { return false; }
        if !self.mapping.tap_dance.contains_key(td_key) { return false; }

        // fire_switch 设 td_action 经 resolve_output 解析字段（hold/double_hold）+ emit_output 发送。
        // 无需在此手工读 output/层名。
        self.fire_switch(td_key, kind);
        self.remove_waiting_stack_item(td_key);
        self.set_td_state(td_key, match kind {
            SwitchKind::Hold => TdKind::Holding,
            SwitchKind::DoubleHold => TdKind::DoubleHolding,
        });
        true
    }

    /// _TryDeferLayerInterrupt: 判断当前键是否应被层键 TD 延迟
    /// 1. 用 target_anchor_layer 确定当前键应查哪一层 → 该层有 TD 则不延迟
    /// 2. 需要延迟时，以 waiting_stack 栈顶键作为 defer anchor（修饰键也可作为 anchor）
    /// 3. waiting_stack 为空 → 不延迟
    pub fn try_defer_layer_interrupt(&mut self, ctx: &Context) -> bool {
        let key = &ctx.key;

        // 栈空 → 无键可做 defer anchor → 不延迟
        if self.state.defer.waiting_stack.is_empty() { return false; }

        // 用 target_anchor_layer 取当前键应查的层
        let anchor_layer = match self.target_anchor_layer(key) {
            Some((_, l)) => l,
            None => return false,
        };

        // 检查在当前层是否有 TD → 有 TD 不延迟
        if let Some(km) = self.mapping.tap_dance.get(key) {
            if let Some(td_map) = km.get(&anchor_layer) {
                if !td_map.hold.is_empty()
                    || !td_map.double_tap.is_empty()
                    || !td_map.double_hold.is_empty() {
                    return false; // 目标层有 TD 定义 → 不延迟
                }
            }
        }

        // 需要延迟：使用栈顶键作为 defer anchor（修饰键也可以）
        let top_key = self.state.defer.waiting_stack.last().map(|(k, _)| k.clone());
        if let Some(tk) = top_key {
            self.state.defer.keys_deferred.insert(key.to_string(),
                DeferInfo { td_key: tk });
            true
        } else {
            false
        }
    }

    /// _FlushDeferredEntry: 重入延迟条目，取消 TD 定时器，经 P6 ForceHold 激活 waiting 切换键
    pub fn flush_deferred_entry(&mut self, key: &str) {
        if !self.state.defer.keys_deferred.contains_key(key) { return; }
        let info = self.state.defer.keys_deferred.remove(key).unwrap();
        let td_key = info.td_key.clone();

        if let Some(s) = self.state.keys.key_states.get(&td_key).and_then(|__ks| __ks.td.as_ref()) {
            if s.kind != TdKind::Finished {
                self.cancel_timer(&td_key, TimerKind::Hold);
                self.cancel_timer(&td_key, TimerKind::DoubleTap);
                self.cancel_timer(&td_key, TimerKind::DoubleHold);
            }
        }

        // 统一写入 re_ctx.force_hold_down（触发键=被 flush 的延迟键，非层键故不在 waiting_stack → 走「全激活」；
        // 若延迟键本身是层键(在栈中)则激活其之前(不含自己)。anchor(td_key) 通常位于栈底，
        // 「全激活」必含 anchor → 其层上下文正确，与旧 [..=idx] 行为一致（单 anchor 场景完全相同）。
        let mut re_ctx = Context::new(key);
        re_ctx.td_action = TdAction::Tap;
        re_ctx.force_hold_down = Some(key.to_string());
        self.re_enter(&mut re_ctx, FLUSH_DEFERRED_REENTER);
    }

    /// _FireSwitch: 用虚拟上下文触发层激活（通过 re_enter → Phase7）
    /// phys_key: 触发该层键的物理键名（用于 phase8_final 把层键记入 KeyState.emitted[phys_key]）
    /// fire_switch: 走统一 resolve+output 管道激活层/修饰键。
    /// 设 td_action 由 resolve_output 解析 hold/double_hold 字段。
    /// 不再手工组装 logical_key，彻底消除绕过 resolve 的路径。
    pub fn fire_switch(&mut self, td_key: &str, kind: SwitchKind) {
        self.debug("LAYER", &format!("fireSwitch td_key={} kind={:?}", td_key, kind));
        let mut ctx = Context {
            key: td_key.to_string(),
            td_action: match kind {
                SwitchKind::Hold => TdAction::Hold,
                SwitchKind::DoubleHold => TdAction::DoubleHold,
            },
            ..Default::default()
        };
        self.re_enter(&mut ctx, COMMIT_SEND_REENTER);
    }

}
