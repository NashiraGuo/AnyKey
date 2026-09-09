/// AnyKey engine — Leader 系统（leader key / 序列匹配）
///
/// 拦截+记录钩子（commit_stage 插入点）、滑动超时、匹配算法、执行器。
/// 详见 leader-plan.md。

use crate::state::*;
use crate::util::*;

/// Leader 滑动超时计时器使用的哨兵 key（全局单例，cancel/reschedule 共用）
const LEADER_TIMER_KEY: &str = "__leader__";
/// Leader 触发键——硬编码，任何键的 hold/dbl-tap 输出写 {leader} 即可成为屏蔽触发键
const LEADER_TRIGGER: &str = "{leader}";

impl PipelineState {
    /// Leader 拦截 + 记录钩子（Step3）：commit_stage 插入点调用（keydown P7 与 keyup up5 共用）。
    /// 只有真实会发出的键才到达此处（output 之前的短路已过滤空值）。
    pub(crate) fn leader_intercept_record(&mut self, ctx: &mut Context) {
        // 未启用 / 无序列 → 完全旁路，不影响既有行为
        if self.mapping.leader.sequences.is_empty() { return; }

        // leader action 自身合成键（__leader_*）：默认不参与拦截/记录（避免阻塞续等时自噬或污染栈）；
        // 开启 loop_capture（循环捕获）时放行，使输出键回灌驱动串联。
        if ctx.key.starts_with("__leader_") && !self.config.leader.loop_capture {
            return;
        }

        let physical = ctx.key.clone();
        let logical = ctx.logical_key.clone();

        // (0) 层键豁免：不拦截、不记录、不进栈（output 照常激活层，层键走自身 keyup 反激活）
        if is_layer_key(&logical) {
            self.debug("LEADER", &format!("record phys={} logical={} intercepted=false [layer-key exempt]", physical, logical));
            return;
        }
        // (0') 空输出（combo 首键等无 logical）→ 空转
        if logical.is_empty() {
            self.debug("LEADER", &format!("record phys={} logical=<empty> intercepted=false [empty output]", physical));
            return;
        }

        let blocking = !LEADER_TRIGGER.is_empty();

        // (1) 已拦截键（blocking 拦截态内已置 leader_blk 的本键）→ 不二次记录、不二次拦截。
        //     TD 键 keyup 会再进 commit_stage，此时 leader_blk 已置、output 已短路，
        //     此处跳过记录避免重复压栈。
        if self.leader_blk(&ctx.key) {
            self.debug("LEADER", &format!("record phys={} logical={} intercepted=true [already-blk]", physical, logical));
            return;
        }

        // (2) 触发键 {leader}（仅 blocking / trigger 非空）
        if blocking && !self.state.leader.leader_on && logical == LEADER_TRIGGER {
            self.state.leader.leader_on = true;
            self.state.leader.leader_stack.clear();
            self.state.leader.leader_pending = None;
            self.state.leader.leader_pending_phys = None;
            self.leader_touch_timeout();   // 重置滑动超时窗口（避免上一条序列残留计时器误触发）
            self.set_leader_blk(&ctx.key);
            self.debug("LEADER", &format!("record phys={} logical={} intercepted=true [trigger on]", physical, logical));
            return;  // 自身不进栈
        }

        // (3) blocking 拦截态：本键被吞（置 leader_blk），但仍记录 logical 进栈
        if blocking && self.state.leader.leader_on {
            self.set_leader_blk(&ctx.key);
            if is_single_key(&logical) {
                self.state.leader.leader_stack.push((ctx.key.clone(), logical.clone()));
                self.leader_touch_timeout();
            }
            self.debug("LEADER", &format!("record phys={} logical={} intercepted=true [blk-seq push={}]", physical, logical, is_single_key(&logical)));
            return;  // output() 因 leader_blk 短路，不实际发送
        }

        // (4) non-blocking（含 blocking 未激活时）：照发，仍记录 + 刷新超时
        if is_single_key(&logical) {
            self.state.leader.leader_stack.push((ctx.key.clone(), logical.clone()));
            self.leader_touch_timeout();
        }
        // 不置 leader_blk → output() 正常 emit
        self.debug("LEADER", &format!("record phys={} logical={} intercepted=false [nonblk record]", physical, logical));
    }

    /// Leader 滑动超时刷新（Step6）：取消旧的 LeaderTimeout，重新调度到 tick + leader_timeout_ms。
    /// 每次压栈都调用 → 形成「滑动窗口」：超时只看距上次按键的时间。
    /// `short` 是否为 `long` 的前缀（元素逐一相等，且长度不超过 long）
    fn seq_is_prefix(short: &[String], long: &[String]) -> bool {
        if short.len() > long.len() { return false; }
        short == &long[..short.len()]
    }

    /// 当前滑动窗口应使用的超时（毫秒）：
    /// 当前栈是某序列前缀时，取这些「候选序列」里 `timeout_ms` 最大的那条；
    /// 候选都没设专属超时（timeout_ms==0）则回退公共 `leader_timeout_ms`。
    fn leader_active_timeout(&self) -> u64 {
        let stack: Vec<String> = self.state.leader.leader_stack.iter().map(|(_, l)| l.clone()).collect();
        let mut best: Option<u64> = None;
        for s in &self.mapping.leader.sequences {
            if Self::seq_is_prefix(&stack, &s.keys) && s.timeout_ms > 0 {
                best = Some(match best { Some(b) => b.max(s.timeout_ms), None => s.timeout_ms });
            }
        }
        best.unwrap_or(self.mapping.leader.timeout_ms)
    }

    fn leader_touch_timeout(&mut self) {
        if self.mapping.leader.sequences.is_empty() { return; }
        let active = self.leader_active_timeout();
        self.cancel_timer(LEADER_TIMER_KEY, TimerKind::LeaderTimeout);
        self.schedule_timer(TimerKind::LeaderTimeout, LEADER_TIMER_KEY, "", active);
    }

    /// Leader 匹配（§6）：读 `leader_stack`，按目的「dead-end 清栈 / 未完成等待 / 匹配最长序列」结算。
    /// 在 `phase_up_leader` 的 (b) 调用；结果写入 `leader_pending` / `leader_pending_phys`。
    fn leader_match(&mut self) {
        if self.mapping.leader.sequences.is_empty() { return; }
        // 评估前清空上次 pending（每次 keyup / 级联都重新评估，避免陈旧 pending 残留影响执行判定）
        // 空栈时保留 pending（可能由前一次 exact 匹配产生但另一键触发等待被执行）；
        // 非空才重新评估并清旧 pending
        let stack = self.state.leader.leader_stack.clone();                 // Vec<(phys, logical)>
        let stack_keys: Vec<String> = stack.iter().map(|(_, l)| l.clone()).collect();
        if stack_keys.is_empty() { return; }

        self.state.leader.leader_pending = None;
        self.state.leader.leader_pending_phys = None;

        // 分析阶段：只读 self.mapping.leader.sequences，结果收为拥有值（避免与下方可变 self 冲突）
        let mut exact_output: Option<String> = None;
        let mut exact_len: usize = 0;
        let mut is_proper_prefix = false;
        for s in &self.mapping.leader.sequences {
            if s.keys.len() < 2 { continue; }   // 防御：序列至少 2 键（配置层已过滤，此处兜底，避免 1 键序列误匹配）
            if s.keys.len() == stack_keys.len() && s.keys == stack_keys {
                // exact：取最长（greedy 最长）
                if exact_output.is_none() || s.keys.len() > exact_len {
                    exact_output = Some(s.output.clone());
                    exact_len = s.keys.len();
                }
            } else if s.keys.len() > stack_keys.len() && s.keys[..stack_keys.len()] == stack_keys {
                // stack 是某序列的真前缀 → 还需等更多键
                is_proper_prefix = true;
            }
        }

        // 决策阶段：可变 self
        if let Some(out) = exact_output {
            // exact 命中 → 立即触发（最长精确匹配）
            self.state.leader.leader_pending = Some(out);
            self.state.leader.leader_pending_phys = stack.last().map(|(p, _)| p.clone());
            if is_proper_prefix {
                // 当前栈仍是更长序列的前缀 → 保留栈、继续等后续键延伸
                // 例如 {1}{2} 已触发 {3}，但 {1}{2}{3} 存在 → 保留 [1,2] 等 '3' 续接出 {4}
                return;
            }
            // 不是任何更长序列的前缀 → 触发完毕，清栈
            self.state.leader.leader_stack.clear();
            return;
        }
        if is_proper_prefix {
            return; // 未完成 → 等待（保留栈，压栈时已刷新超时）
        }

        // dead-end：既非 exact 也非任何序列前缀 → 清栈
        let last = stack.last().cloned();   // (phys, logical)
        self.state.leader.leader_stack.clear();
        // non-blocking 额外 re-feed：末键若是某序列首键，作为新起点重压栈再匹配一次
        // （必落到「等待」分支，自然终止，不会无限递归）；否则直接清栈（原键已照发，无需补救）
        if LEADER_TRIGGER.is_empty() {
            if let Some((lp, ll)) = last {
                let can_start = self.mapping.leader.sequences.iter().any(|s| s.keys.get(0) == Some(&ll));
                if can_start {
                    self.state.leader.leader_stack.push((lp, ll));
                    self.leader_touch_timeout();
                    self.leader_match();   // 递归：stack=[ll] 必为某序列前缀 → 等待分支返回
                }
            }
        } else {
            // blocking dead-end：同时退出拦截态，避免用户被卡住只能等超时
            self.state.leader.leader_on = false;
        }
    }

    /// Leader 匹配执行器（Step4）：挂在 `key_up_inner` 末尾（`phase_up8_clean` 之后），
    /// 由 `key_up` / `reenter_up` 统一经 `key_up_inner` 执行（reenter 路径亦覆盖）。
    pub(crate) fn phase_up_leader(&mut self, ctx: &mut Context) {
        if self.mapping.leader.sequences.is_empty() { return; }

        self.debug("LEADER", &format!(
            "phase_up key={} logical={} stack=[{}]",
            ctx.key, ctx.logical_key,
            self.state.leader.leader_stack.iter().map(|(p, l)| format!("{}/{}", p, l)).collect::<Vec<_>>().join(" | ")
        ));

        // (b) 匹配：只读 leader_stack（记录器内容），不读 ctx.logical_key（用户第 E 点）
        self.leader_match();

        // (c) 执行：仅当本键是触发匹配的栈顶键（pending_phys == ctx.key）
        if let Some(out) = self.state.leader.leader_pending.clone() {
            if self.state.leader.leader_pending_phys.as_deref() == Some(&ctx.key) {
                self.leader_execute(&out, &ctx.key);    // 见 §7，经 commit_stage 发送
                // 退拦截检查放在 execute 之后：cascade 可能在 execute 内清栈，
                // 若在 execute 前检查，cascade 路径的栈尚未清空会漏关 leader_on。
                if !LEADER_TRIGGER.is_empty() && self.state.leader.leader_stack.is_empty() {
                    self.state.leader.leader_on = false;
                }
                self.state.leader.leader_pending = None;
                self.state.leader.leader_pending_phys = None;
            }
        }
    }

    /// Leader 执行（§7）：直接调用 `commit_stage` 发送 action 输出（输出自包含 down+up）。
    fn leader_execute(&mut self, output: &str, _phys: &str) {
        self.leader_execute_depth(output, 0, &None);
    }

    /// `leader_execute` 带级联深度参数版本（供循环捕获递归调用）。
    /// `last_out`：上一级已执行的输出，用于防自复制配置（如 12→12）无限级联。
    fn leader_execute_depth(&mut self, output: &str, depth: usize, last_out: &Option<String>) {
        const MAX_LOOP_DEPTH: usize = 64;   // 硬上限：任何配置下级联都不超过 64 层，防失控
        if depth >= MAX_LOOP_DEPTH {
            self.debug("LEADER", &format!("cascade depth limit reached, stop output={}", output));
            return;
        }
        // 合成键名作 emitted 索引，避免污染完成键自身 td/combo 状态（一次性 token）。
        let synth = format!("__leader_{}", output);
        let mut actx = Context::new(&synth);
        actx.logical_key = output.to_string();
        // leader 输出全部自包含（send_key 对 __leader_* 走 emit_tap_si / emit_text / emit_run，
        // 不经 Down/Up 事件通道、不记 emitted），无需 release_key 补发 Up。
        // 合成键是一次性 token：用后即焚删除其 key_state（防御性清理，正常路径从未创建该条目）。
        self.commit_stage(&mut actx);
        self.state.keys.key_states.remove(&synth);
        self.debug("LEADER", &format!("execute OK output={}", output));

        // 循环捕获：本次输出入栈后重新匹配并执行，驱动序列自动串联（带硬上限防失控）。
        if self.config.leader.loop_capture {
            let next_out = Some(output.to_string());
            self.leader_cascade(depth + 1, &next_out);
        }
        let _ = last_out;   // 仅级联侧使用
    }

    /// 循环捕获级联：synth 键已入栈（loop_capture 时 bypass 放行），重新评估栈；
    /// 若本轮 leader_match 产生新 pending 则继续递归执行，直到无新触发 / 触及深度上限 / 自复制死循环。
    fn leader_cascade(&mut self, depth: usize, last_out: &Option<String>) {
        const MAX_LOOP_DEPTH: usize = 64;
        if depth >= MAX_LOOP_DEPTH { return; }
        // leader_match 已在开头清空 pending，故调用后若 pending 非空，必为本轮新匹配结果。
        self.leader_match();
        if let Some(out) = self.state.leader.leader_pending.clone() {
            // 防自复制配置（如 12→12，多键输出不被入栈 → 栈不增长 → 持续重匹配同一输出）：
            // 若本次输出与上级相同，说明无新进展，终止级联（避免 64 次空转）。
            if Some(&out) == last_out.as_ref() {
                self.state.leader.leader_pending = None;
                self.state.leader.leader_pending_phys = None;
                return;
            }
            let next_out = out.clone();
            self.state.leader.leader_pending = None;
            self.state.leader.leader_pending_phys = None;
            self.leader_execute_depth(&next_out, depth, &Some(next_out.clone()));
        }
    }
    /// Leader 滑动超时回调（Step6）：超时到期 → 清栈 + 清 pending。
    /// blocking 额外退拦截（leader_on=false）；non-blocking 无拦截概念。
    /// 注意：leader_blk 由各键 keyup 时 phase_up_leader 清理，此处不动（保留至抬起，避免幽灵 Up）。
    pub fn leader_timeout(&mut self) {
        if self.mapping.leader.sequences.is_empty() { return; }
        self.state.leader.leader_stack.clear();
        self.state.leader.leader_pending = None;
        self.state.leader.leader_pending_phys = None;
        if !LEADER_TRIGGER.is_empty() {
            self.state.leader.leader_on = false;   // blocking 退拦截，本次序列作废
        }
    }
}
