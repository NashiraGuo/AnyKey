// pipeline/commit.rs — commit 系统：解析 + 发送 + 清理
//   resolver（logical_key 解析）/ output / commit_stage（P7 + Up5 入口）
//   send_key / release_key（Down/Up 发送统一出口）+ record_emitted + is_valid_key_name
//   emit_* 发送通道（键盘/文本/鼠标/层事件）+ activate/deactivate_layer
use crate::state::*;
use crate::util::*;
use super::*;

impl PipelineState {
    /// 公共 resolver：解析 logical_key（合并 keydown 的 phase7 与 keyup 的 phase5 重复映射 + combo 匹配）。
    /// 仅负责「怎么解析映射」；「是否进入解析（flags）」「combo 优先（仅 keydown）」仍由 phase 层负责。
    /// 内部先处理 combo 匹配（按 combo.state 分 Matched / 未完成组 / Holding），再处理普通 td_action 映射：
    ///   - Matched（keydown）：层感知查表解析 combo 输出，写双方 output + 转 Holding/Active；
    ///   - Failed/Stuck/Seeking/Interrupted（combo 未匹配）：退化为本键自身 td_action 解析（不 return）；
    ///   - Holding：keydown 重复（单键 combo）→ 直接取 combo.output；
    ///   - Active/Released：combo 输出由 release_key 按 combo.output 发送，不走普通解析（_ 兜底空输出）。
    /// 层感知查表 / 释放值回落（emitted → base.hold → resolve_key_output）等细节内联在此，
    /// keydown 与 keyup 共用同一套映射规则。各 td_action 只在其所属方向的管道被设置，
    /// 反向不会出现（如 keydown 不会出现 HoldRelease / keyup 不会出现 Intercept·Hold·Holding），
    /// 故合并为单一 match 安全——未触发的分支为死代码但不影响行为。
    /// 注：keydown 的 Seeking/Interrupted 由 phase3 的 SILENCE 跳过发送（但 resolver 仍被调用），
    /// 故 Matched 仅 keydown、Failed/Stuck 仅 keyup 触发，统一在此处理无冲突。
    pub(super) fn resolver(&mut self, ctx: &mut Context) {
        // ── combo 匹配（keydown 与 keyup 统一）──
        if let Some(combo) = self.combo(&ctx.key).cloned() {
            match combo.state {
                ComboKind::Matched => {
                    let partner = combo.partner.clone();
                    let output = self.resolve_combo_output(&ctx.key, &partner);
                    if !output.is_empty() {
                        ctx.logical_key = output.clone();
                        if let Some(s) = self.state.keys.key_states.get_mut(&ctx.key).and_then(|ks| ks.combo.as_mut()) {
                            s.output = output.clone();
                        }
                        if let Some(s) = self.state.keys.key_states.get_mut(&partner).and_then(|ks| ks.combo.as_mut()) {
                            s.output = output.clone();
                        }
                        let fs = if is_single_key(&output) { ComboKind::Holding } else { ComboKind::Active };
                        for k in [&ctx.key, &partner] {
                            if let Some(s) = self.state.keys.key_states.get_mut(k).and_then(|ks| ks.combo.as_mut()) {
                                s.state = fs.clone();
                            }
                        }
                        return;
                    }
                    // 极端：查不到输出 → 仍转 Holding，不阻塞
                    for k in [&ctx.key, &partner] {
                        if let Some(s) = self.state.keys.key_states.get_mut(k).and_then(|ks| ks.combo.as_mut()) {
                            s.state = ComboKind::Holding;
                        }
                    }
                    return;
                }
                // combo 未完成（Failed/Stuck/Seeking/Interrupted）：本键按自身 td_action 解析，交给下方 tapdance 部分。
                // - Failed/Stuck 来自 keyup 管道（phase_up1_combo 把 Seeking 转 Failed）。
                // - Seeking 来自 keydown 管道「孤 combo 键」：伙伴未按下，phase3 标 Seeking + SILENCE 跳过发送，
                //   但 resolve_output 仍会被调用（phase7 不查 SILENCE），需落此臂退化为本键 tap（被 SILENCE 抑制）。
                // - Interrupted 来自 keydown 管道「第三键打断」：combo_interrupt_all 把本键 Seeking→Interrupted 后
                //   经 re_enter(COMBO_TIMEOUT_REENTER) 重跑管道并调用 resolve_output（等伙伴中、被中断）。
                //   四种状态都是「combo 未匹配」，本键应退化为自身 td_action 输出；不再强制退化成 tap、不再 return。
                //   （注：Seeking/Interrupted 在 keyup 已被 phase_up1_combo 转 Failed，此处仍能收到二者仅因 keydown 路径；
                //    故不能 panic —— 否则每次第三键打断 combo / 按下孤 combo 键都会让引擎崩溃。
                //    test_combo_interrupt_by_third_key 覆盖 Interrupted 路径。）
                ComboKind::Failed | ComboKind::Stuck | ComboKind::Seeking | ComboKind::Interrupted => {
                    // 空臂：不 return，执行流继续到 if-let 之后由 td_action → logical_key 解析
                }
                ComboKind::Holding => {
                    // keydown 重复（单键 combo）：从 combo.output 设 logical_key
                    // 设完后 return，不落到下方 td_action 解析（否则会被 tap 值覆写）
                    ctx.logical_key = combo.output.clone();
                    return;
                }
                _ => {
                    // Active/Released：combo 输出由 release_key 发送，不走普通解析
                    return;
                }
            }
        }

        // ── 普通 td_action → logical_key 解析（非 combo）──
        // 键不在 keys 表中时（如 {sleepMacro} 等合成键），不解析、保持 caller 已设的 logical_key。
        // resolve_key_output 依赖 keys 表查 tap/hold/dt/dh，键不存在时无意义。
        if !self.mapping.tap_dance.contains_key(&ctx.key) { return; }
        // td_action 由 tap_dance_down 确定：None 表示等待（defer），不在此归一化。
        if ctx.td_action != TdAction::None {
            match &ctx.td_action {
                TdAction::Intercept => { ctx.logical_key.clear(); }
                TdAction::Tap => {
                    // 经 keydown 重入（re_enter）发出的单 tap（dt_timer / 延迟键 / 中断补发）
                    // 需解析配置 tap，不能落到 _ 兜底保留 Context::new 的「键自身」默认值。
                    let rpl = self.resolve_key_output(&ctx.key, "tap");
                    if !rpl.is_empty() { ctx.logical_key = rpl; }
                }
                TdAction::Hold => {
                    let hld = self.resolve_key_output(&ctx.key, "hold");
                    if !hld.is_empty() { ctx.logical_key = hld; }
                    else {
                        let rpl = self.resolve_key_output(&ctx.key, "tap");
                        if !rpl.is_empty() { ctx.logical_key = rpl; }
                    }
                }
                TdAction::Holding => {
                    // 从 layer_stack 顶层：若本键激活了层，用存的原值做 BN/TN/FN 判定
                    let mut handled = false;
                    if let Some(entry) = self.state.layers.layer_stack.last() {
                        if entry.activated_by == ctx.key {
                            handled = true;
                            if entry.resolved_output.to_lowercase().starts_with("{bn")
                               || entry.resolved_output.to_lowercase().starts_with("{tn") {
                                ctx.logical_key.clear();          // BN/TN → 拦截（tn 与 bn 一致，按住重复不输出）
                            } else {
                                let rpl = self.resolve_key_output(&ctx.key, "tap");
                                if !rpl.is_empty() { ctx.logical_key = rpl; }  // FN → 重复 tap
                            }
                        }
                    }
                    // 修饰键或非层键 → 旧路径
                    if !handled {
                        if self.is_switch_key(&ctx.key) {
                            let hld = self.resolve_key_output(&ctx.key, "hold");
                            if !hld.is_empty() {
                                ctx.logical_key = hld;
                            }
                        } else {
                            ctx.logical_key.clear();
                        }
                    }
                }
                TdAction::TwoTap => {
                    let rpl = self.resolve_key_output(&ctx.key, "tap");
                    ctx.logical_key = if !rpl.is_empty() {
                        format!("{}{}", rpl, rpl)
                    } else {
                        format!("{{{}}}{{{}}}", ctx.key, ctx.key)
                    };
                }
                TdAction::DoubleTap => {
                    // double_tap 值已在构建映射表时归一化为 {X}（键）或保留为文本。
                    // 无 doublehold 的第二次按下确认（state=DoubleTapHold）走此分支：
                    // 键 → phase8 emit_down 按住、emitted 记录，物理键 UP 时释放；
                    // 文本 → phase8 emit_text 立即 down+up（无法按住，符合预期）。
                    let rpl = self.resolve_key_output(&ctx.key, "doubleTap");
                    if !rpl.is_empty() { ctx.logical_key = rpl; }
                }
                TdAction::DoubleHold => {
                    let rpl = self.resolve_key_output(&ctx.key, "doubleHold");
                    if !rpl.is_empty() { ctx.logical_key = rpl; }
                }
                TdAction::HoldRelease => {
                    // 释放值优先取 phase8_final 记录的 emitted（完整 {X} 形式），不再读瞬态 ctx.hold_value
                    let mut hld = if let Some(em) = self.emitted(&ctx.key).cloned() { em }
                                   else { String::new() };
                    if hld.is_empty() {
                        if let Some(k) = self.mapping.tap_dance.get(&ctx.key) {
                            if let Some(base) = k.get("base") {
                                if !base.hold.is_empty() { hld = base.hold.clone(); }
                            }
                        }
                    }
                    if hld.is_empty() { hld = self.resolve_key_output(&ctx.key, "hold"); }
                    if !hld.is_empty() {
                        ctx.logical_key = hld;
                    } else {
                        let rpl = self.resolve_key_output(&ctx.key, "tap");
                        if !rpl.is_empty() { ctx.logical_key = rpl; }
                    }
                }
                TdAction::DoubleHoldRelease => {
                    let mut rpl2 = if let Some(em) = self.emitted(&ctx.key).cloned() { em } else { String::new() };
                    if rpl2.is_empty() {
                        if let Some(k) = self.mapping.tap_dance.get(&ctx.key) {
                            if let Some(base) = k.get("base") {
                                if !base.double_hold.is_empty() { rpl2 = base.double_hold.clone(); }
                            }
                        }
                    }
                    if rpl2.is_empty() { rpl2 = self.resolve_key_output(&ctx.key, "doubleHold"); }
                    if !rpl2.is_empty() { ctx.logical_key = rpl2; }
                }
                _ => {} // None 已在入口归一为 Tap，不会到达；保留以满足穷尽性
            }
        } else {
            // 不可达：td_action 已在入口兜底归一为 Tap。保留 tap 解析作最终防线。
            let rpl = self.resolve_key_output(&ctx.key, "tap");
            if !rpl.is_empty() { ctx.logical_key = rpl; }
        }
    }

    // ═══════════════════════════════════════════
    // Resolve+Output 统一模块
    // ═══════════════════════════════════════════

    /// output: 公共输出阶段。只负责「逻辑键 → 物理发送」。
    /// keydown / keyup 共用。不含 flag 门控——调用方负责。
    /// 守卫链：
    ///   1) 空输出 → skip
    ///   2) 层键已激活 → skip（target_layer == logical_key 说明 keydown 已激活，keyup 不重复）
    pub(super) fn output(&mut self, ctx: &mut Context) {
        // Leader 拦截：本键被吞 → 不 emit（Step3）。
        // __leader_ 合成键是 action 自身的输出，始终放行发送（避免阻塞续等时自噬，或循环捕获时吞掉回灌键）。
        if self.leader_blk(&ctx.key) && !ctx.key.starts_with("__leader_") { return; }
        if ctx.logical_key.is_empty() { return; }
        if is_layer_key(&ctx.logical_key)
            && self.target_layer(&ctx.key).map_or(false, |t| t == &ctx.logical_key) {
            return;
        }
        self.send_key(ctx);
    }

    /// commit_stage: 顺序执行 resolver → output，之间预留新 Phase 插入点。
    /// 不含 flag 门控——调用方在 wrapper phase 中负责 flag 判断与标记。
    pub(crate) fn commit_stage(&mut self, ctx: &mut Context) {
        // Phase A：键值解析（resolver）
        self.resolver(ctx);

        // ── Leader 拦截 / 记录钩子（Step3）──
        // 此处 ctx.logical_key 已是最终 remap 结果（resolver 之后、output 之前）。
        // keydown(P7) 与 keyup(up5) 共用本钩子；只有真实会发出的键才到达此处。
        self.leader_intercept_record(ctx);

        // Phase B：最终输出
        self.output(ctx);
    }

    /// 验证 {X} 花括号内容是否为预设的合法键名（键盘/鼠标/媒体/浏览器）。
    /// 单字符 {a}/{1} 始终合法；多字符需在 scancode 或鼠标键名表中有映射。
    pub(super) fn is_valid_key_name(inner: &str) -> bool {
        let kn = crate::util::canonical_key_name(inner);
        kn.chars().count() == 1
            || crate::emit::key_name_to_scancode(&kn).is_some()
            || crate::emit::is_mouse_key_name(&kn)
    }

    // ── 输出控制 ──
    /// send_key: 仅负责「怎么发」——按 logical_key 类型决定发送方式并统一记录。
    /// 「需不需要发」由上层 phase 过滤（空输出 / 层键 keyup 不发送），本函数不再判断。
    ///
    /// 单一职责：
    /// - 层键 → activate_layer + 记 target_layer（仅 keydown 路径经上层放行到达此处，故无需 is_keydown 分支）
    /// - 单键 {KeyName} → emit_down + 记 emitted（Up 由 release_key 发）
    /// - 文本 / 宏 / 鼠标 / 函数 → emit_text/emit_run/emit_mouse_move 自包含 + 记 emitted（release 仅清不重发）
    /// - {Sleep N} → 前缀立即发（递归走统一路径）、后缀经 SleepTimer 延迟
    pub fn send_key(&mut self, ctx: &mut Context) {
        // 通道栅栏计数器（2026-09-10）：每次输出执行新建一个 FLT 事件计数，
        // 子递归（Sleep/Select/Repeat 的前后缀）经 &Cell 共享同一计数。
        // 普通打字是独立的小执行、早已排空，与本计数无关（这是与全局计数器的
        // 本质区别：栅栏值 = 本执行内自最近通道切换以来的 FLT 积压，精确无污染）。
        let flt = std::cell::Cell::new(0u64);
        self.send_key_inner(ctx, &flt);
    }

    fn send_key_inner(&mut self, ctx: &mut Context, flt: &std::cell::Cell<u64>) {
        let output = ctx.logical_key.clone();
        let key = ctx.key.clone();
        // 从 key_state 取触发设备的 ID（key_down 入口写入），不依赖 self.current_device
        let device_id = self.state.keys.key_states.get(&ctx.key)
            .map(|ks| ks.device_id)
            .unwrap_or(0);

        // 层键：激活层 + 记 target_layer（反激活由 release_key 读 target_layer 完成；上层 keyup 已过滤不调用本函数）
        if is_layer_key(&output) {
            // {tn0}：清空整个 layer_stack（切回 base），每个 entry 发一次 LayerOff；不 push、不记 target_layer。
            if output.to_lowercase() == "{tn0}" {
                let names: Vec<String> = self.state.layers.layer_stack.iter().map(|e| e.name.clone()).collect();
                self.state.layers.layer_stack.clear();
                for n in names { self.emit_layer_deact(&n); }
                return;
            }
            let action = match &ctx.td_action {
                TdAction::Tap => {
                    // combo Matched 的非 TD 键也被 tap_dance_down 设成了 Tap → 矫正为 combo
                    if self.state.keys.key_states.get(&ctx.key).and_then(|ks| ks.combo.as_ref())
                        .map_or(false, |c| c.state == ComboKind::Matched)
                    {
                        "combo"
                    } else {
                        "tap"
                    }
                }
                TdAction::Hold         |
                TdAction::Holding      |
                TdAction::HoldRelease  => "hold",
                TdAction::DoubleTap    => "doubleTap",
                TdAction::DoubleHold   => "doubleHold",
                _                      => "combo",
            };
            self.activate_layer(&extract_layer_name(&output), &ctx.key, action, &output);
            if !key.is_empty() {
                self.set_target_layer(&key, extract_layer_name(&output));
            } else if let Some((wk, _)) = self.state.defer.waiting_stack.last().cloned() {
                self.set_target_layer(&wk, extract_layer_name(&output));
            }
            return;
        }

        // {Sleep N} — 非阻塞延迟：前缀立即发出（走统一路径含记录），后缀经 SleepTimer 延迟发送。
        // 放在 leader 单键处理之前，使 leader 输出中的 {Sleep 100} 也能被正常识别。
        let sleep_re = regex_lazy(r"(?i)(.*?)\{Sleep (\d+)\}(.*)");
        if let Some(caps) = sleep_re.captures(&output) {
            let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let delay: u64 = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
            let suffix = caps.get(3).map(|m| m.as_str()).unwrap_or("");
            if !prefix.is_empty() {
                // 递归发送前缀（走统一路径，含记录；FLT 发射计入本执行的栅栏计数）
                let mut sub = ctx.clone();
                sub.logical_key = prefix.to_string();
                self.send_key_inner(&mut sub, flt);
            }
            if !suffix.is_empty() && delay > 0 {
                self.debug("TIMER", &format!("Schedule SleepTimer key={} delay={}", key, delay));
                let deadline = self.tick + delay;
                let sleep_id = self.next_timer_id();
                self.timer_heap.push(Reverse(TimerEntry {
                    id: sleep_id,
                    deadline_tick: deadline,
                    kind: TimerKind::SleepTimer,
                    key: if key.starts_with("__leader_") { "__leader_{sleepMacro}".to_string() } else { "{sleepMacro}".to_string() },
                    td_key: if key.starts_with("__leader_") { "__leader_{sleepMacro}".to_string() } else { "{sleepMacro}".to_string() },
                    extra_data: suffix.to_string(),
                    device_id: self.current_device,
                    mapping: self.mapping.clone(),
                }));
            }
            return;
        }

        // {Select N} — 向左选中 N 个字符（快捷宏）：{Left N}{shift down}{Right N}{shift up}。
        // 支持前缀和后缀（如 hira@126.com{Select 12} → 先发文本，再选中）。
        // 放在 {KeyName N} 之前，避免 {Select 12} 被当成 12 次 "select" 键。
        // 按键段统一走 FLT 通道（无 injected 标记，不会被 IME/LL 钩子按合成键吞掉导致
        // shift 卡死；A/B 实测 FLT 同样零间隔 shift 不卡）。文本前缀/后缀仍走 SI
        // （unicode 只有 SI 能发）。单一通道边界：文本(SI 整批入队) → 按键(FLT)，
        // SendInput 返回时文本已入队，FLT 不超车。
        let select_re = regex_lazy(r"(?i)(.*?)\{Select\s+(\d+)\}(.*)");
        if let Some(caps) = select_re.captures(&output) {
            let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let count: usize = caps.get(2).unwrap().as_str().parse().unwrap_or(1);
            let suffix = caps.get(3).map(|m| m.as_str()).unwrap_or("");
            if !prefix.is_empty() {
                let mut sub = ctx.clone();
                sub.logical_key = prefix.to_string();
                self.send_key_inner(&mut sub, flt);
            }
            self.debug("SELECT", &format!("Select {} (prefix={:?} suffix={:?})", count, prefix, suffix));
            // FLT burst 事件数：4N+2（left DN/UP ×N + shift DN + right DN/UP ×N + shift UP）
            flt.set(flt.get() + (4 * count + 2) as u64);
            for _ in 0..count {
                self.emit_down("left", device_id);
                self.emit_up("left", device_id);
            }
            self.emit_down("shift", device_id);
            for _ in 0..count {
                self.emit_down("right", device_id);
                self.emit_up("right", device_id);
            }
            self.emit_up("shift", device_id);
            if !suffix.is_empty() {
                // 通道栅栏：FLT burst 引擎瞬间发完，但系统以 ~1.0ms/事件被 RIT/LL
                // 钩子节流排空（实测）；SI 文本无队列背压，会插进还在排空的 FLT 流
                // → 字符交错（实测 {Select 12}+suffix → 654321）。按本执行内积压的
                // FLT 事件数精确估算 sleep（1.0 实测 + 20% 余量 + 2ms 固定）。
                // 后缀实际会走 SI 才需要：leader 恒 SI；非 leader 简单 ASCII 走 FLT，
                // 同通道无需栅栏（is_simple_text 含 { 的后缀偏保守多睡，方向安全）。
                self.channel_fence(flt, key.starts_with("__leader_") || !Self::is_simple_text(suffix));
                let mut sub = ctx.clone();
                sub.logical_key = suffix.to_string();
                self.send_key_inner(&mut sub, flt);
            }
            return;
        }

        // {KeyName N} — 重复发送 N 次（如 hira@126.com{left 12} → 先发文本，再 Left 键 tap 12 次）。
        // 支持前缀和后缀（与 {Sleep N} / {Select N} 相同模式）。
        #[allow(clippy::single_char_pattern)]
        let repeat_re = regex_lazy(r"(?i)(.*?)\{([^\s}]+)\s+(\d+)\}(.*)");
        if let Some(caps) = repeat_re.captures(&output) {
            let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let key_name = caps.get(2).unwrap().as_str();
            let count: usize = caps.get(3).unwrap().as_str().parse().unwrap_or(1);
            let suffix = caps.get(4).map(|m| m.as_str()).unwrap_or("");
            if !prefix.is_empty() {
                let mut sub = ctx.clone();
                sub.logical_key = prefix.to_string();
                self.send_key_inner(&mut sub, flt);
            }
            self.debug("REPEAT", &format!("{} x{}", key_name, count));
            // 非 leader 的重复 tap 走 FLT（鼠标键走鼠标通道不计）：计入栅栏计数；
            // leader 走 TapSI（SI 同通道）不计。
            if !key.starts_with("__leader_") && !crate::emit::is_mouse_key_name(key_name) {
                flt.set(flt.get() + (2 * count) as u64);
            }
            for _ in 0..count {
                if key.starts_with("__leader_") {
                    self.emit_tap_si(key_name);  // SendInput 自包含 tap
                } else {
                    self.emit_down(key_name, device_id);    // Down/Up 逐次 tap
                    self.emit_up(key_name, device_id);
                }
            }
            if !suffix.is_empty() {
                // 通道栅栏：非 leader 重复 tap（FLT）后接 SI 后缀（大写/中文等）时，
                // 前缀 FLT + taps 的积压会在 SI 文本发出前精确排空。leader 全 SI 不触发
                // （pending=0）；后缀为简单 ASCII 时走 FLT 同通道也不触发。
                self.channel_fence(flt, key.starts_with("__leader_") || !Self::is_simple_text(suffix));
                let mut sub = ctx.clone();
                sub.logical_key = suffix.to_string();
                self.send_key_inner(&mut sub, flt);
            }
            return;  // 自包含，不记 emitted
        }

        // Leader 合成键（__leader_*）：纯单键（{b}/{1}/{enter}/{f1}…）→ SendInput 扫描码 tap。
        // 自包含、不记 emitted。单字符与控制键名同走扫描码 tap（单字符可尊重修饰键状态
        // 如 Shift；控制键名含 E0）。放在 {Sleep N} 之后：leader 输出中的 {Sleep 100}
        // 由上层 Sleep 分支处理。
        if key.starts_with("__leader_") && is_single_key(&output) {
            let raw = strip_slot_suffix(&output[1..output.len() - 1]);
            self.emit_tap_si(&raw);
            return;
        }

        // RUN:cmd — ShellExecute 执行（自包含，记 emitted 供 leader 漏斗）
        if let Some(cmd) = output.strip_prefix("RUN:") {
            self.emit_run(cmd);
        }
        // MouseMove(x, y) — 相对鼠标移动（自包含）
        else if let Some(caps) = regex_lazy(r"(?i)^MouseMove\(([^,]+),\s*([^,]+)\)$").captures(output.trim()) {
            let x: i32 = caps.get(1).and_then(|m| m.as_str().trim().parse().ok()).unwrap_or(0);
            let y: i32 = caps.get(2).and_then(|m| m.as_str().trim().parse().ok()).unwrap_or(0);
            self.emit_mouse_move(x, y, device_id);
        }
        // 函数调用 (FuncName()) → emit_text 原样输出（自包含）
        else if is_func_call(&output) {
            self.emit_text(&output);
        }
        // 单键 {KeyName} → EmitDown + 记 emitted（完整 {X} 形式），Up 交给 release_key
        else if is_single_key(&output) {
            let key_name = strip_slot_suffix(&output[1..output.len()-1]);
            if Self::is_valid_key_name(&key_name) {
                // 鼠标键走独立通道
                if crate::emit::is_mouse_key_name(&key_name) {
                    self.emit_mouse_down(&key_name, device_id);
                } else {
                    self.emit_down(&key_name, device_id);
                }
            } else {
                self.debug("SEND", &format!("UNKNOWN KEY: {{{}}} (not a valid key name)", key_name));
            }
        }
        // 分段输出：解析所有 {keyname}、{keyname down}、{keyname up} 模式。
        // 纯键名 → Down/Up 事件通道（emit_down/emit_up 不记 emitted — 自包含 down+up）；
        // 含文本 / leader 合成键 → 统一 SendInput 通道
        // （emit_tap_si / emit_down_si / emit_up_si / emit_text），不混通道。
        else {
            struct Seg<'a> { start: usize, end: usize, key: &'a str, action: &'a str }
            let re = regex_lazy(r"\{([^}]+)\}");
            let mut segs: Vec<Seg> = Vec::new();
            let mut last_end = 0;
            let mut bare_len: usize = 0;
            for cap in re.captures_iter(&output) {
                let m = cap.get(0).unwrap();
                let inner = cap.get(1).unwrap();
                if m.start() > last_end {
                    bare_len += m.start() - last_end;
                }
                let raw = inner.as_str();
                // 单字符 {a}/{1} 保留为文本（二义性：可能是宏 literal）
                if raw.chars().count() <= 1 {
                    bare_len += m.len();
                    last_end = m.end();
                    continue;
                }
                let (key_s, action) = if let Some(k) = raw.strip_suffix(" down") {
                    (k, "down")
                } else if let Some(k) = raw.strip_suffix(" up") {
                    (k, "up")
                } else {
                    (raw, "tap")
                };
                segs.push(Seg { start: m.start(), end: m.end(), key: key_s, action });
                last_end = m.end();
            }
            if last_end < output.len() {
                bare_len += output.len() - last_end;
            }

            // 通道选择：leader 始终 SendInput；其他情况优先 Down/Up 事件通道，
            // 但若文本包含不可逐字符发送的字符（大写字母/Shift 符号/中文等）→ SendInput
            let use_si = if key.starts_with("__leader_") {
                true
            } else if bare_len > 0 {
                // 收集所有裸文本片段，检查是否可逐字符发送
                let mut all_text = String::new();
                let mut t_last = 0;
                for seg in &segs {
                    if seg.start > t_last { all_text.push_str(&output[t_last..seg.start]); }
                    t_last = seg.end;
                }
                if t_last < output.len() { all_text.push_str(&output[t_last..]); }
                // 若 segments 为空，整段输出就是文本
                if segs.is_empty() { all_text = output.clone(); }
                !Self::is_simple_text(&all_text)
            } else {
                false  // 纯键名 → Down/Up 事件通道
            };

            // Emit each segment
            last_end = 0;
            for seg in &segs {
                if seg.start > last_end {
                    self.emit_seg_text(&output[last_end..seg.start], use_si, flt);
                }
                match seg.action {
                    "down" if use_si => self.emit_down_si(seg.key),
                    "down" => { self.emit_down(seg.key, device_id); self.set_emitted(seg.key, seg.key.to_string()); flt.set(flt.get() + 1); }
                    "up" if use_si => self.emit_up_si(seg.key),
                    "up" => { self.emit_up(seg.key, device_id); flt.set(flt.get() + 1); }
                    "tap" if use_si => {
                        if seg.key.chars().count() == 1 {
                            self.emit_bare_text(seg.key, use_si);
                        } else {
                            self.emit_tap_si(seg.key);
                        }
                    }
                    "tap" => {
                        self.emit_down(seg.key, device_id);
                        self.emit_up(seg.key, device_id);
                        flt.set(flt.get() + 2);
                    }
                    _ => {}
                }
                last_end = seg.end;
            }
            // 剩余文本
            if !segs.is_empty() && last_end < output.len() {
                self.emit_seg_text(&output[last_end..], use_si, flt);
            }

            // 无任何 {keyname} 模式 → 纯文本
            if segs.is_empty() {
                self.emit_seg_text(&output, use_si, flt);
                if use_si {
                    // SendInput 文本：走下方 record_emitted 供 leader 漏斗
                } else {
                    return;  // Down/Up 文本已自包含
                }
            } else {
                return;  // 有分段：已自包含，跳过下方 record_emitted
            }
        }

        // 记录 emitted（非分段路径）：单键 / RUN / MouseMove / func_call
        self.record_emitted(&key, &output);
    }

    /// 记录 emitted（非层键）：优先用物理键，wait_stack 顶作为 fallback（历史 phase_up5 行为）。
    fn record_emitted(&mut self, key: &str, logical: &str) {
        if !key.is_empty() {
            self.set_emitted(key, logical.to_string());
        } else if let Some((wk, _)) = self.state.defer.waiting_stack.last().cloned() {
            self.set_emitted(&wk, logical.to_string());
        }
    }

    /// _ReleaseKey: 释放物理键（phase_up6_release 末尾调用，所有发送的统一出口）
    ///   1) combo 模式：output 仍按住时跳过；若本键 pending_up（最后抬起）则用 combo.output 发一次 Up，
    ///      发完立即 clear_emitted/clear_target_layer 清掉本键残留；非本键负责的 combo 键静默跳过。
    ///   2) target_layer（层键，仅非 combo）：反激活目标层一次（与 emitted 解耦，方案3）
    ///   3) emitted（单键/媒体/鼠标，仅非 combo）：发 Up；Run/MouseMove/文本 fire-and-forget 不发 Up
    pub fn release_key(&mut self, physical_key: &str) {
        // Leader 拦截：被吞键的 Up 不发送（Step3）；其清理由 phase_up_leader 负责
        if self.leader_blk(physical_key) { return; }
        self.debug("SEND", &format!("releaseKey physical={}", physical_key));
        let device_id = self.state.keys.key_states.get(physical_key)
            .map(|ks| ks.device_id)
            .unwrap_or(0);
        // 本函数职责 = 仅发送（Up / 层反激活）。所有状态清理（emitted / target_layer /
        // combo 簿记）一律交给 phase_up7_clean，使「发送在本函数、清理在 up8」分层清晰，
        // 且 flushbehind（位于 release 与 clean 之间）能在 K 状态清除前 flush 背后延迟键。
        if let Some(combo) = self.state.keys.key_states.get(physical_key).and_then(|__kc| __kc.combo.as_ref()) {
            if combo.state == ComboKind::Holding {
                return; // combo 仍按住，不发送
            }
            if combo.state == ComboKind::Released {
                let pending = combo.pending_up;
                let output = combo.output.clone();
                if pending {
                    // 本键是 combo 中最后抬起 → 用 combo.output 统一发一次 Up（含层键反激活）。
                    // 与下方非 combo 的 emitted/target_layer 通道互斥（发完即 return，不落到该分支）。
                    // 层键反激活的 activated_by：触发键抬起用自己，伙伴键抬起用 partner（= 触发键）。
                    // 判定依据：target_layer 只记在触发键上（send_key 里 set_target_layer 只对触发键写）。
                    let activated_by = if self.target_layer(physical_key).is_some() {
                        physical_key.to_string()        // 触发键抬起
                    } else {
                        combo.partner.clone()           // 伙伴键抬起 → partner = 触发键
                    };
                    self.emit_up_for_output(&output, device_id, &activated_by);
                }
                return; // 非本键负责（伙伴仍按住）→ output 保持 / 不发送
            }
        }
        // 层键 target_layer：反激活目标层一次（层无实体键，不发物理 Up）
        if let Some(target) = self.target_layer(physical_key).cloned() {
            self.deactivate_layer(physical_key, &target);
        }
        // 单键 / 媒体键 / 鼠标键：发 Up（去掉花括号与槽后缀）
        if let Some(em) = self.emitted(physical_key).cloned() {
            if is_single_key(&em) {
                let kn = strip_slot_suffix(&em[1..em.len()-1]);
                if Self::is_valid_key_name(&kn) {
                    if crate::emit::is_mouse_key_name(&kn) {
                        // 滚轮是瞬时事件，不发送 UP（DN 已自包含滚动数据）
                        let is_wheel = matches!(kn.to_lowercase().as_str(),
                            "wheelup" | "wheeldown" | "wheelleft" | "wheelright");
                        if !is_wheel {
                            self.emit_mouse_up(&kn, device_id);
                        }
                    } else {
                        self.emit_up(&kn, device_id);
                    }
                }
                // 非预设键名：已由 send_key 报错并丢弃，此处不发 Up
            }
            // 其它（Run/MouseMove/文本 func）：fire-and-forget，不发 Up
            // 反激活层的功能已移到上方 target_layer 处理，emitted 不再承载层键。
            // clear_emitted / clear_target_layer 由 phase_up8_clean 完成。
        } else if is_single_key(physical_key) {
            // 兜底：emitted 未记录但 physical_key 本身是单键形式（延迟键等）
            let kn = &physical_key[1..physical_key.len()-1];
            self.emit_up(kn, device_id);
        }
    }

    /// EmitDown: 推入按键按下事件
    pub fn emit_down(&mut self, kn: &str, device_id: u32) {
        self.debug("SEND", &format!("EmitDown: {}", kn));
        let kn = strip_slot_suffix(kn);
        self.emit_log.push(EmitEvent::Down(kn, device_id));
    }

    /// EmitUp: 推入按键释放事件
    pub fn emit_up(&mut self, kn: &str, device_id: u32) {
        self.debug("SEND", &format!("EmitUp: {}", kn));
        let kn = strip_slot_suffix(kn);
        self.emit_log.push(EmitEvent::Up(kn, device_id));
    }

    /// EmitText: 推入文本事件（宏/函数调用）
    pub fn emit_text(&mut self, txt: &str) {
        self.debug("SEND", &format!("EmitText: {}", txt));
        self.emit_log.push(EmitEvent::Text(txt.to_string(), self.current_device));
    }

    /// EmitTapSI: 经 SendInput 扫描码注入的 tap（含 E0 扩展键）。自包含 down+up，
    /// 不记 emitted、不进 release_key。由 leader 合成键的控制键名（{enter}/{end}/{f1}…）
    /// 分支使用，使其与 Text(unicode) 同处 SendInput/RIT 有序输入流，避免与
    /// Down/Up 事件通道交错。
    pub fn emit_tap_si(&mut self, kn: &str) {
        self.debug("SEND", &format!("EmitTapSI: {}", kn));
        self.emit_log.push(EmitEvent::TapSI(kn.to_string(), self.current_device));
    }

    /// EmitDownSI: 经 SendInput 扫描码注入的 key down（按住不释放），由 {key down} 语法使用。
    /// 与 emit_tap_si / emit_text 同处 SendInput/RIT 有序流。
    pub fn emit_down_si(&mut self, kn: &str) {
        self.debug("SEND", &format!("EmitDownSI: {}", kn));
        self.emit_log.push(EmitEvent::DownSI(kn.to_string(), self.current_device));
    }

    /// EmitUpSI: 经 SendInput 扫描码注入的 key up（释放），由 {key up} 语法使用。
    pub fn emit_up_si(&mut self, kn: &str) {
        self.debug("SEND", &format!("EmitUpSI: {}", kn));
        self.emit_log.push(EmitEvent::UpSI(kn.to_string(), self.current_device));
    }

    /// 将 ASCII 纯文本逐字符经 Down/Up 事件发送（字符→down→up 逐次循环）。
    /// 仅用于小写字母/数字/基础符号——无需 Shift 组合可直接发送。
    /// 大写字母/Shift 符号/非 ASCII（中文等）应走 SendInput，不能调用此函数。
    pub fn emit_text_charwise(&mut self, txt: &str) {
        self.debug("SEND", &format!("EmitTextCharwise: {}", txt));
        for ch in txt.chars() {
            let s = ch.to_string();
            self.emit_down(&s, 0);
            self.emit_up(&s, 0);
        }
    }

    /// 检查文本是否仅包含可直接逐字符发送的简单 ASCII（小写字母/数字/空格/基础符号）
    fn is_simple_text(txt: &str) -> bool {
        txt.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()
            || matches!(c, ' ' | '`' | '-' | '=' | '[' | ']' | '\\' | ';' | '\'' | ',' | '.' | '/'))
    }

    /// 通道栅栏（FLT→SI 切换点专用）：FLT 事件引擎瞬间发完（IOCTL 非阻塞），但系统以
    /// ~1.0ms/事件被 RIT/LL 钩子节流排空（2026-09-10 keylatency 实测；SI unicode
    /// ~1.19ms/事件）。SI→FLT 方向有队列背压天然保护（SendInput 队列满→阻塞发射线程），
    /// FLT→SI 方向无背压，SI 文本会插进还在排空的 FLT 流 → 字符交错（实测 {Select 12}
    /// + suffix → 654321）。时长 = 本执行内积压 FLT 事件数 × 1.2ms + 2ms，作为 Fence
    /// 指令推入 emit_log。
    ///
    /// ⚠️ 不能在这里 sleep：commit 只做决策、把事件写进 emit_log，真正发射在 main 的
    /// drain 阶段。在此 sleep 会睡在【所有输出之前】（前缀 FLT burst 还没提交给驱动），
    /// 白等一场 —— 2026-09-10 实测确认。必须让指令随 emit_log 排到 FLT burst 之后、
    /// 后缀 SI 之前，由 drain 执行。
    ///
    /// `need`=后缀实际会走 SI（leader 恒真 / 非 leader 看后缀是否简单 ASCII）。
    /// 入队后清零计数——栅栏之后积压视为已排空，本执行内后续 FLT 从零计。
    fn channel_fence(&mut self, flt: &std::cell::Cell<u64>, need: bool) {
        let pending = flt.get();
        if !need || pending == 0 { return; }
        let ms = pending * 12 / 10 + 2;
        self.debug("FENCE", &format!("FLT->SI switch, {} pending FLT events, fence {}ms", pending, ms));
        self.emit_log.push(EmitEvent::Fence(ms));
        flt.set(0);
    }

    /// 分段器文本发射：FLT 逐字符路径计入栅栏计数（每字符 down+up = 2 事件），
    /// SI 路径不计（SI 自身积压由背压保护，且 SI→FLT 不需要栅栏）。
    fn emit_seg_text(&mut self, text: &str, use_si: bool, flt: &std::cell::Cell<u64>) {
        if !use_si {
            flt.set(flt.get() + 2 * text.chars().count() as u64);
        }
        self.emit_bare_text(text, use_si);
    }

    /// 发送裸文本片段：use_si=true 走 SendInput API；false 走 Down/Up 事件通道。
    /// SendInput 模式下单字符走 emit_tap_si（扫描码，尊重修饰键状态如 Shift），
    /// 多字符/非 ASCII 走 emit_text（unicode 文本，绕开修饰键状态）。
    fn emit_bare_text(&mut self, text: &str, use_si: bool) {
        if use_si {
            if text.chars().count() == 1 && Self::is_simple_text(text) {
                self.emit_tap_si(text);  // 单字符扫描码 → 受 Shift 等修饰键影响
            } else {
                self.emit_text(text);    // 多字符/非 ASCII → unicode 文本
            }
        } else {
            self.emit_text_charwise(text);
        }
    }

    /// EmitRun: 推入 ShellExecute 命令
    pub fn emit_run(&mut self, cmd: &str) {
        self.debug("SEND", &format!("EmitRun: {}", cmd));
        self.emit_log.push(EmitEvent::Run(cmd.to_string(), self.current_device));
    }

    /// EmitMouseMove: 推入鼠标相对移动事件
    pub fn emit_mouse_move(&mut self, x: i32, y: i32, device_id: u32) {
        self.debug("SEND", &format!("EmitMouseMove: {},{}", x, y));
        self.emit_log.push(EmitEvent::MouseMove(x, y, device_id));
    }

    /// EmitMouseDown: 推入鼠标键按下事件（pipeline 已判定为鼠标键）
    pub fn emit_mouse_down(&mut self, name: &str, device_id: u32) {
        self.debug("SEND", &format!("EmitMouseDown: {}", name));
        self.emit_log.push(EmitEvent::MouseDown(name.to_string(), device_id));
    }

    /// EmitMouseUp: 推入鼠标键释放事件
    pub fn emit_mouse_up(&mut self, name: &str, device_id: u32) {
        self.debug("SEND", &format!("EmitMouseUp: {}", name));
        self.emit_log.push(EmitEvent::MouseUp(name.to_string(), device_id));
    }

    /// EmitLayerAct: 推入层激活事件（0→1 跃迁）
    pub fn emit_layer_act(&mut self, name: &str) {
        self.debug("LAYER", &format!("EmitLayerOn: {}", name));
        self.emit_log.push(EmitEvent::LayerOn(name.to_string(), 1));
    }

    /// EmitLayerDeact: 推入层反激活事件（→0 跃迁）
    pub fn emit_layer_deact(&mut self, name: &str) {
        self.debug("LAYER", &format!("EmitLayerOff: {}", name));
        self.emit_log.push(EmitEvent::LayerOff(name.to_string(), 1));
    }

    /// emit_up_for_output: 按 combo output 字符串释放（与 release_key 的反激活/Up 逻辑一致，
    ///   但作用于任意 output 字符串，不依赖 emitted）。层键→反激活；单键→Up；Run/文/鼠标→无 Up
    /// `activated_by`：反激活层时用于精确定位 layer_stack entry（combo 场景下触发键/伙伴键均可能抬起）。
    pub fn emit_up_for_output(&mut self, output: &str, device_id: u32, activated_by: &str) {
        if is_layer_key(output) {
            self.deactivate_layer(activated_by, &extract_layer_name(output));
        } else if is_single_key(output) {
            let kn = strip_slot_suffix(&output[1..output.len()-1]);
            if crate::emit::is_mouse_key_name(&kn) {
                let is_wheel = matches!(kn.to_lowercase().as_str(),
                    "wheelup" | "wheeldown" | "wheelleft" | "wheelright");
                if !is_wheel {
                    self.emit_mouse_up(&kn, device_id);
                }
            } else {
                self.emit_up(&kn, device_id);
            }
        }
        // 其它（Run/MouseMove/文本 func）：fire-and-forget，无 Up
    }

    /// _ActivateLayerImpl: 压栈（去计数器后，激活 = push，无引用计数）。
    /// LayerOn 每次压栈都发（过程记录，嵌套激活同一层会连续出现多个事件，属正常）。
    pub fn activate_layer(
        &mut self,
        name: &str,
        activated_by: &str,
        action: &str,
        resolved_output: &str,
    ) {
        // tn 幂等：当前键（activated_by）已在 layer_stack 中 → 不重复压栈。
        // 用 activated_by 检查而非「name == current_layer」：
        //   - 场景「先 fn1 再 tn1 再都松开」：fn1 键先压 entry(fn键)，tn1 键再压 entry(tn键)。
        //     若按 current_layer 判断会误判「已在 fn1」而跳过压栈，fn1 键松开后 fn1 完全消失，
        //     期望是 tn1 独立压一个 entry 固定住 fn1。
        //   - 用 activated_by 检查：tn 键首次触发时栈里无自己的 entry → 正常压栈；
        //     该键再次触发（auto-repeat / 重复 hold）时自己的 entry 还在 → 跳过，不塞栈。
        // 仅针对 tn（keyup 不反激活的永久层）；fn/bn 是临时层，嵌套压栈是必要的
        // （去计数器后靠 (activated_by, name) 精确出栈，每个键的 entry 独立反激活）。
        if resolved_output.to_lowercase().starts_with("{tn")
            && self.state.layers.layer_stack.iter().any(|e| e.activated_by == activated_by) {
            self.debug("LAYER", &format!("skip +{} (activated_by {} already in stack, tn idempotent)", name, activated_by));
            return;
        }
        self.debug("LAYER", &format!("+{} stack=[{}] by={} action={}",
            name,
            self.state.layers.layer_stack.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join("|"),
            activated_by, action));
        self.state.layers.layer_stack.push(LayerEntry {
            name: name.to_string(),
            activated_by: activated_by.to_string(),
            action: action.to_string(),
            resolved_output: resolved_output.to_string(),
        });
        self.emit_layer_act(name);
    }

    /// _DeactivateLayerImpl: 按 (activated_by, name) 精确出栈（去计数器后，反激活 = pop 对应 entry）。
    /// LayerOff 每次出栈都发。tn 层（resolved_output 以 {tn 开头）keyup 静默，不反激活。
    pub fn deactivate_layer(&mut self, activated_by: &str, name: &str) {
        self.debug("LAYER", &format!("-{} stack=[{}]", name,
            self.state.layers.layer_stack.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join("|")));
        let Some(idx) = self.state.layers.layer_stack.iter()
            .position(|e| e.activated_by == activated_by && e.name == name) else { return; };
        // {tnX} toggle 层：keyup 不反激活（区别于 {fnX}/{bnX} 临时层）
        if self.state.layers.layer_stack[idx].resolved_output.to_lowercase().starts_with("{tn") {
            return;
        }
        self.state.layers.layer_stack.remove(idx);
        self.emit_layer_deact(name);
    }

}
