# AnyKey 引擎 · 工作节点记录（WORKLOG）

> 用途：记录引擎重构的阶段性节点（已完成项 + 待办 + 架构反思），跨会话以本文件 + `leader-plan.md` 为准。
> 约定：每个节点用日期 + 一句话标题锚定；新增节点追加在文件头部。

---

## 节点 2026-07-10 凌晨 — phase_up2_combo 拆分 + resolver 彻底合并（combo 匹配全并入 resolve_output）

### 已完成
1. **phase_up2_combo 拆分（只写状态/flags）**：
   - `ComboKind::Waiting`：仅 `force_hold_switch_key` 仍按住的 waiting switch 键（层/修饰，保证 fallback tap 用正确层解析，原职责保留）+ `ctx.flags &= !(F_UP4 | F_UP5_OUTPUT)` 路由到 resolver→output→release；**删除原 `re_enter(COMBO_TIMEOUT_REENTER)`**（发送职责移出本阶段，交后续 phase）。清理（clear_combo / pending_timers.remove）交 up6。
   - `ComboKind::Stuck`：空块（不写状态），清理交 up6。
   - 其余（Holding/Active/Released）：保持 `resolve_key_up` 标 Released + pending_up，不清理（发送/清理由 release_key 完成）。
2. **resolver 彻底合并（combo 匹配全进公共 resolve_output）**：
   - `resolve_output` 顶部新增 combo 匹配块：`Matched`（层感知查表写双方 output、转 Holding/Active）/ `Waiting`（keyup 退化本键自身 tap）/ `Stuck`（清 logical_key）/ `_`（Holding/Active/Released：combo 输出由 release_key 发送，不在此解析）。
   - `phase7_resolve`（keydown）删原 combo 优先块，改为 `self.resolve_output(ctx); ctx.flags |= F_RESOLVE;`。
   - keydown 的 Waiting 不会到达此处（phase3 `SILENCE` 跳过 phase7），故 Matched 仅 keydown、Waiting/Stuck 仅 keyup，统一处理无冲突。
3. **phase_up6_release combo 收尾扩展为 match**：
   - `Released`：本键与伙伴都 Released 才清（原逻辑）。
   - `Waiting | Stuck`：清本键+伙伴 combo + 移除 pending timer。
   - `_`：不处理。

### 行为变化分析（供参考；用户判定 golden 是 spec，此变化须通过改管道达成，而非改 golden）
- 删除 Waiting 分支的 `re_enter` 后，**combo 键抬起不再 re-enter keydown 管道**：
  - 双按 combo 键不再经 TD 状态机合并为 `TwoTap`（每个按下独立发自身 tap）。
  - 按住 combo 键（hold 已触发）抬起不再 re-fire hold（仅释放已按住的 hold）——**与非 combo hold 键行为一致**（auto-repeat 的第二次 DN 在 `pipeline.rs:1050` 被 Holding 分支吸收，本就不重发）。
- 即 combo 键未命中伙伴时，行为收敛为「与普通键一样发自身绑定」，不再是 re-enter 驱动的合并/重发。这是用户设计的预期结果。
- 备份：`backups/anykey-engine.pre-20260710-combo-up2`。

### ⚠️ 更正（2026-07-10 上午）— golden 已恢复，管道仍待定
- **用户判定**：golden 是 spec，管道没改完时测试就该红，不该反过来改 golden。我先前把 6 个 golden 重新生成为「新行为」是错的（假绿）。
- **已恢复**：6 个 golden 还原为原始（`backups/tests/scenarios/*.json` 是重生前拷贝），`td_hold_repeat_shift.desc` 也还原。
- **现状**：这 6 个 scenario 测试现在是红的（`cargo test --test scenario_test` → 45 passed / 6 failed），正是「管道未完成」该有的信号，不是 stale golden。
- 先前「78 项全绿」的结论**撤回**——那是改了 spec 换来的，无效。
- **待解的管道问题**：删除 Waiting 分支的 `re_enter` 后，combo 键未命中伙伴时不再走 TD 合并（`TwoTap` 宏）也不重发 hold；但 spec（原始 golden）要求这些行为保留。需重新设计 `phase_up2_combo` 的 Waiting 路由，使 combo 键未命中时仍走 TD 合并 / hold 重发，同时不破坏「resolver 合并」「phase 只写状态」的架构收敛。可能方向：在 resolver/output 层补回「Waiting combo 键按 TD 状态解析自身绑定」的逻辑，而非 re-enter 整条 keydown 管道。

### 验证（当前）
- `cargo test --test scenario_test`：45 passed / 6 failed（6 个 combo+TD 场景为红，符合预期 signal）。
- 其他 scenario（含非 combo 的 hold/tap/layer/defer）全过，说明 phase 拆分 / resolver 合并的代码对普通键正确，问题集中在 combo Waiting 路由。

### 待办（回来后）
- [ ] **管道重做**：让 combo 键未命中伙伴时仍保留 TD 合并 / hold 重发行为（恢复 re_enter 等价效果，但放在正确的 phase 位置），使 6 个 golden 重新转绿。
- [ ] 合并后与 `leader-plan.md` Step0 协同评审（输出口唯一化是否自动达成，phase_leader 插入点是否简化）。

---

## 节点 2026-07-09 深夜 — keydown/keyup 的 resolver+output 应合并为公共模块

### 一、本节点已完成（截至此刻）

1. **keyup 阶段拆分 + flags 门控**
   - `phase_up5_final` 拆为 `phase_up5_output`（发送）+ `phase_up6_release`（释放 + 清理）。
   - `key_up` 调用顺序：`up1 → up2 → up3 → up4 → up5_output → up6_release`。
   - 各 phase 顶部 `if ctx.flags & F_UPx != 0 { return; }` 门控。

2. **`needs_output` 彻底用 flags 取代**
   - `key_up` 入口默认 `flags = F_UP4 | F_UP5_OUTPUT`（=「纯释放」默认路径）。
   - 需现发新输出的 3 处（up1 延迟键 tap、up3 Waiting→Tap、up3 DoubleWait→DoubleTap/TwoTap）清掉这两个 skip 标志，行为等价旧 `needs_output=true`。
   - `Context` 删 `needs_output` 字段。

3. **`release_only` 字段彻底删除**
   - 确认它仅是 debug 标记，从未参与门控（门控全靠 `ctx.flags`）。
   - 删 4 处写点 + 1 处 debug 读点 + 相关注释字眼改为「释放路径」。

4. **`reenter_up` + `fire_sleep_timer`（计时器输出口收紧）**
   - `reenter_up(key)`：构造 ctx 跳过 up1~up5，只跑 `phase_up6_release`（release_key 发 Up + clear_emitted/clear_td/clear_combo）+ 补 `clear_td` 覆盖非 Finished 态。
   - `fire_sleep_timer(entry)`：合成键 `{sleepMacro}` 经 `re_enter(SEND_REENTER)` 走 phase8 发 Down（记 emitted），再 `reenter_up` 同键清理。
   - `dt_timer` 改为 `re_enter(RESOLVE_SEND_REENTER)` 发 tap + `reenter_up(key)` 等价清理（取代旧「clear_td + release_key 空操作」）。
   - `state.rs` / `main.rs` 两处 `TimerKind::SleepTimer` dispatch 同步改 `fire_sleep_timer`。

5. **BUG 修复：`phase7_resolve` 缺 `TdAction::Tap` 分支**
   - 现象：带 `dt` 的键只点一次超时 → `dt_timer` 经 `re_enter` 发键自身 `{1}` 而非配置 tap `{a}`。
   - 根因（已实证，且修正了早先「预存 gap」的误判）：旧代码（2026-07-07 备份）用 `phase6_resolve`（**String** match + 通用兜底 `_ => resolve_key_output(key, action)`），"tap" 靠兜底解析；`reenter_up` 重构把 `td_action` 改枚举、把 `phase6_resolve` 重写为枚举版 `phase7_resolve` 时，**通用兜底被替换成空 `_ => {}`**，"Tap" 没补成显式臂 → 发键自身。
   - 即：**此 bug 是 reenter_up 重构期引入**，不是项目自始就有。
   - 修复：补 `TdAction::Tap` 臂（与 `phase_up4_resolve` 对齐）；测试断言收紧为 `vec!["Down(a)","Up(a)"]`。

6. **验证**：`cargo test` 全绿（28：26 单元 + reachability + scenario golden，含 `doubletap_hold_test` 5 项），零 warning。

### 二、新反思（用户插入）：keydown / keyup 的 resolver + output 应合并为公共模块

**触发**：整理完 keyup 后，发现 keydown、keyup 两边经过重构已经各自拥有一组「解析器 + 输出器」，且两者高度同构。

**现状对照**

| 通道 | 解析器（resolver） | 输出器（output） | 释放/清理 |
|---|---|---|---|
| keydown | `phase7_resolve` | `phase8_final`（发 Down、记 emitted） | — |
| keyup | `phase_up4_resolve` | `phase_up5_output`（发 tap/send_key） | `phase_up6_release`（发 Up + 清 emitted/td/combo） |

**核心洞察**
- `phase8_final` 本质上就是一个 **output 阶段**；keydown 的「resolver → output」与 keyup 的「resolver → output」是同一结构的两份拷贝。
- 同时维护两套模块不合理。**应把这两组合并成一组「公共 resolver + 公共 output」，供 keydown 与 keyup 两边共用。**
- 这与 `leader-plan.md` 的 **Step0 reenter-tightening（统一输出口）** 同源 —— 合并后所有输出（keydown 原发 / keyup 经 reenter / 定时器经 reenter）天然汇聚到同一个公共 output，leader 的记录/拦截点也只需挂一处。

**合并后预期形态（待细化）**
- 公共 `resolve_output(ctx)`：替换 `phase7_resolve` 与 `phase_up4_resolve` 里重复的 `td_action → logical_key` 映射（这正是上一轮 dt_timer bug 的根源 —— 两份独立维护的解析逻辑分叉）。
- 公共 `emit_output(ctx, is_keydown)`：替换 `phase8_final` 与 `phase_up5_output` 的发送逻辑，统一 `emit_down` + 记 `emitted`（keydown 经 `is_keydown=true` 激活层键，keyup 经 `is_keydown=false`）；`two_tap` 的 `{X}{X}` 与文本·宏·鼠标走 `send_key`（自包含，不记 emitted）。
- keyup 专属的「释放 + 清理」（发 Up、清 emitted/td/combo）作为 keyup 收尾单独保留或参数化，不混进公共 output 的核心发射逻辑。
- keydown / keyup 的差异收敛为「方向（Down/Up）+ 是否释放清理」，解析与发射共享。

**实施进度**
- ✅ **output 合并已落地（2026-07-09 22:5x）**：`emit_output(ctx)` 抽出为公共函数，分支覆盖「层键 / 单键 / 文本·宏·鼠标」全部内容；由 `ctx.emit_mode`（`EmitMode::{Down,Tap}`，默认 Down）区分 keydown/keyup 语义：
  - `phase8_final`（keydown 层）：保留 flags / `clear_combo` / `td_pending` 门控，置 `emit_mode=Down` 后调 `emit_output`。
  - `phase_up5_output`（keyup 层）：保留 `F_UP5_OUTPUT` 门控，置 `emit_mode=Tap` 后调 `emit_output`。
  - 行为等价旧实现（keydown 记 emitted + 层键 activate；keyup 发 tap Down+Up 不记 emitted、层键跳过）。`cargo test` 全绿（28）。
- ⬜ **resolver 合并待做（下一步）**：抽 `resolve_output(ctx)` 合并 `phase7_resolve` 与 `phase_up4_resolve` 的 `td_action → logical_key` 映射；phase 层只做 flags/debug/combo 优先级，核心调 `resolve_output`。
- 关键设计点：用户要求「**两组函数合并成一组而非嵌套调用**」，故 `emit_output` 是单一公共函数、phase 层为薄包装（门控 + 设 emit_mode + 调核心），无嵌套调用。

**output 记录的「统一」实验 + 结论（2026-07-09 23:1x）**
- 用户设想：进一步消除 keydown/keyup 差异——`emit_output` 不再做 Down/Tap 分支，**统一记录 emitted（tap 分支也记）**，层键也不再被上层跳过、统一进 `emit_output`。
- **实测：该设想引出 BUG**。把「记 emitted」改成 Down/Tap 都做后，`cargo test` 中 `test_hold_only_doubletap_no_premature_hold` 报双发：`["Down(1)","Up(1)","Up(1)"]`。
  - 根因：keyup tap 经 `emit_output` → `emit_tap`（已发完整 Down+Up 自包含）；若同时记 `emitted`，则 `phase_up6_release` 的 `release_key` 读 `emitted` 会**再发一次 Up** → 双发。普通 switch 键快速点按恰好绕过 up5 未暴露，但 hold-only 双击键走 up5 即炸。
  - 探针结论：差异是**结构性**的——keydown 是「按住」，靠 `emitted` 供后续真实 keyup 的 `release_key` 释放；keyup tap 的 Up 已在 `emit_tap` 内发完，再记 emitted 必双发。无法在公共函数内消除。
  - **设计层定性（用户问「是否设计错误」——是）**：这是**两套「输出生命周期模型」的所有权冲突**，不是写法 bug：
    - 模型 A（keydown 按住 / hold）：先 `emit_down` 发 Down，**把输出挂进 `emitted`**，等待将来某次真实 keyup 的 `release_key` 来发 Up。Down 与 Up **被 `emitted` 桥接、分离在两处**。
    - 模型 B（keyup tap）：`emit_tap` 把 `Down+Up` **一次性自包含发完**，自身已经释放，根本不需要 `emitted`。
    - 实验让 `emit_output` 对两者统一记 `emitted`，等于给模型 B 的「已释放输出」又挂了一份 `emitted` → `release_key` 读它再发一次 Up → **双释放**。所以「tap 也记 emitted」在模型 B 下语义错误。
  - **「按键还按着时发 Down+Up」是误读（已用日志澄清）**：实测 emit 日志为 `Down / Up / Up`（每周期一次 Down + 两次 Up），其中唯一 `Down` 来自 `key_down`（确实在按住时发，正确且必须）；`emit_tap` 概念上也是 Down+Up，但其 `Down` 是「已按住物理键的重复按下」，在事件流中被合并，可见的只是它的 `Up`；**多出来的是 keyup 时刻的第二个 `Up`（来自 release_key），并非「按住时发的一组 Down+Up」**。
  - 实证日志（实验版 `test_hold_only_doubletap` 探针，`key_down/key_up/advance170/key_down/advance60/key_up`）：
    ```
    EMIT: [0]Down("1")  [1]Up("1")  [2]Up("1")  [3]Down("1")  [4]Up("1")  [5]Up("1")
    DBG : Up-Output key=1 logicalKey={1} → EmitTap: 1
          Up-Release key=1 emitted=Some("{1}") → releaseKey → EmitUp: 1   ← 第二个 Up 来自这里
    ```
    直接调用 `emit_tap("1")` 验证其本身推 `Down+Up` 两个事件，排除「tap 不推 Down」的误解。
- **按用户既定 fallback 处理**：记录功能上移回上层——`emit_output` 改为**纯发送**（不再 `set_emitted`/`set_target_layer`），记录（层键 `target_layer`、其余 `emitted`）放回 `phase8_final`（keydown 专属）；`phase_up5_output`（keyup）**不记**。行为回退到正确等价，全测仍 28 绿。
  - 代价：公共函数一体性被削弱（记录分落两层），属用户预判的「被迫选项」。
  - **若未来想真·消除差异**：需改 tap 语义为「up5 只 `emit_down`+记 emitted，Up 一律推迟到 `release_key`(up6) 发」——即 Down/Tap 在 `emit_output` 内完全相同（都 emit_down+记 emitted），Up 全由 release 收口。这是更大改动（影响事件时序），未做，待用户决定。

**Waiting 单拍改用 emit_down 模型（2026-07-10 00:0x，已实施 ✅）**
- 用户反思：`test_hold_only_doubletap_no_premature_hold` 唯独这一例炸，说明「Waiting 抬起的单拍」本就不该用 `emit_tap`（自包含），而应像 keydown 一样用 `emit_down`+记 emitted、`phase_up6_release` 的 `release_key` 发 Up —— 这样与 keydown 的「按住→release 释放」模型一致。
- 落地（`pipeline.rs` `phase_up5_output`）：`emit_mode` 按 `td_action` 选 —— **`TdAction::Tap`（Waiting 单拍）→ `EmitMode::Down`**（emit_down + 记 emitted，Up 交 up6）；其余（`DoubleTap`/`TwoTap` 等）→ `EmitMode::Tap`（emit_tap 自包含，不记 emitted）。
  - 判据精确：全代码仅 `tap_dance_up` 的 `TdKind::Waiting` 分支（:1160）设 `td_action=Tap` 并清 skip 走 up5；`dt_timer`/`phase_up1_deferred`/`:1630` 均为 `re_enter` 走 keydown 管道，不进 up5。故 `td_action==Tap ⟺ Waiting 单拍`。
  - `emit_output` 保持**纯发送**（沿用 fallback）；记录（层键 `target_layer`、其余 `emitted`）上移到 `phase_up5_output`（与 `phase8_final` 逻辑同源，仅 Down 模式记）。
- `double_tap` / `two_tap`：**保持 `emit_tap`**（自包含一次性输出，记了会双发）——不变。
- **延迟键（deferred）已无需改**：`phase_up1_deferred` 的 `TapDone` 分支（:806-814）本就用 `re_enter(Tap)` 走 keydown 管道 → `emit_down`+记 emitted，本就与 keydown 模型一致，不经由 up5 的 `emit_tap`。
- 验证：新增回归测试 `test_waiting_tap_uses_emit_down`（tap="{a}" 单拍）→ 精确 `Down(a)/Up(a)`、无双发、release 后 emitted 清空；全测 28 绿（含该测试为 29 项量级）。
- **最终模型收敛**：
  - 模型 A（按住→release 释放，都走 emit_down+记 emitted）：keydown 原发 / Waiting 单拍 / 延迟键 / dt_timer（均经 keydown 管道或 up5-Down）。
  - 模型 B（自包含一次性，emit_tap 不记）：`double_tap` / `two_tap`（双拍确认输出，本质是已完成的离散事件，不应被「按住」再释放）。
  - 仅模型 B 与 keydown 仍存差异（emit_tap vs emit_down），但这是语义必然（双拍输出是「触发即完成」，不是「按住待释放」），无法也不应统一。

**状态（2026-07-10 00:0x）**：output 合并已完成；Waiting 单拍已统一进模型 A（emit_down+release）；仅 double_tap/two_tap 保留 emit_tap（模型 B，语义必然）。resolver 合并待实施。

**彻底消除 EmitMode::Tap，记录移回公共函数（2026-07-10 00:3x，已实施 ✅）**
- 用户进一步反思（紧接上节）：其实不存在需要 `EmitMode::Tap` 的情形——所有单键输出都能统一进「按住→release 释放」模型：
  - `two_tap`：构建的 `{X}{X}` 宏交给 `emit_output`，因 `is_single_key("{X}{X}")==false` 自然落入 `send_key` 文本宏分支（Down+Up 自包含，fire-and-forget），根本不进 `emit_down`，无需 `emit_tap`。
  - `double_tap` 两个情形：①按下立刻发送（键情形）→ `emit_down` 按住 + 记 emitted，物理键 UP 时 `release_key` 发 Up；②抬起时发送 → `emit_down` 发 Down + 记 emitted，`phase_up6_release` 发 Up。两者都是 `emit_down` 模型，没问题。
  - `Waiting` 单拍：上节已改为 `emit_down` + release。
  - ⇒ 全代码再无「自包含 Down+Up」需求，`emit_tap` 唯一调用点（emit_output 的 Tap 分支）可删，`EmitMode` 枚举与 `ctx.emit_mode` 字段一并移除。
- 落地（`pipeline.rs`）：
  - `emit_output(ctx, is_keydown: bool)`：删 `EmitMode`/`emit_tap` 分支，**统一 `emit_down` + 记录 emitted**；层键仅 `is_keydown` 时 `activate_layer` + 记 `target_layer`，否则 return（反激活由 `release_key` 读 `target_layer`）。`two_tap` 的 `{X}{X}` 与文本·宏·鼠标走 `send_key`（不记 emitted，release 仅清不重发）。**记录（emitted/target_layer）重新回到公共函数内部**，phase 层不再各自记。
  - `phase8_final`：`self.emit_output(ctx, true)`，删除原本地记录块。
  - `phase_up5_output`：`self.emit_output(ctx, false)`，删除 `emit_mode` 选择与本地记录块。
  - 删除 `emit_tap` 辅助函数（无调用点）。
- **结论修正（推翻上节「模型 B 语义必然」）**：上节认为 double_tap/two_tap 是「触发即完成」的离散事件、必须用自包含 emit_tap；现用户指出它们同样可走「按住→release」模型（down 时 emit_down+记 emitted，Up 由 release 收口），net 效果等价（Down 在 up5、Up 在 up6，同一次 keyup 内完成），且彻底消除了 emit_tap 与 release 的双发结构性冲突。故**两套生命周期模型收敛为单一模型 A**，EmitMode::Tap 永久退役。
- 验证：`cargo test` 全绿（含 `test_hold_only_doubletap_no_premature_hold` / `test_waiting_tap_uses_emit_down` / `test_dt_timer_fires_and_cleans_via_reenter_up` 等对记录/双发的锁死）。

**send_key 与 emit_output 融合为单一 send_key（2026-07-10 00:4x，已实施 ✅）**
- 用户指出：`emit_output` 与 `send_key` 都含 `is_single_key` 分支、职责重叠（emit_output 抢了 send_key 的活）。两者融合为一个 `send_key(ctx, is_keydown)`，保留此名。
- 落地（`pipeline.rs`）：
  - 新版 `send_key(&mut self, ctx: &mut Context, is_keydown: bool)`：吸收原 `emit_output` 的**层键处理**（仅 `is_keydown` 时 `activate_layer` + 记 `target_layer`）+ **统一记录 emitted**（新增私有 `record_emitted(key, logical)`：优先物理键，fallback `waiting_stack` 顶）。原 `send_key(output, physical_key)` 的 `{Sleep N}`/`RUN:`/`MouseMove`/函数调用/单键/文本全部分支保留，仅**单一 `is_single_key` 分支**（合并前有两处，现一处）。
  - `{Sleep N}` 前缀递归发送改用 `ctx.clone()` + 改 `sub.logical_key = prefix` 后 `self.send_key(&mut sub, is_keydown)`，使前缀也走统一路径（含记录）；后缀仍经 `SleepTimer` 延迟、合成键 `{sleepMacro}` 收尾。
  - 删除原 `emit_output` 函数；`phase8_final` / `phase_up5_output` 改为 `self.send_key(ctx, true/false)`；两处 phase 层退化为薄包装（flags/debug），不再各自发送或记录。
  - 文本·宏·鼠标（非单键非层）也在函数末尾统一记 `emitted`（与旧 `emit_output` 行为一致：非层输出一律记，release 仅清不重发，不会误发 Up）。
- 测试适配：两处直接 `send_key(output, physical_key)` 的测试改为构造 `Context` 调用 `send_key(&mut ctx, is_keydown)`；`test_sleep_timer_suffix_emitted_and_cleaned` 因前缀 now 记 `emitted("1")="pre"`（fire-and-forget，真实流由 keyup 清除），补 `p.key_up("1")` 还原真实释放后再断言（语义更贴合真实使用）。
- 验证：`cargo test` 全绿（pipeline 26 / doubletap 6 / reachability 1 / scenario 1 / main 5，共 39 项）。

**状态**：output 合并已完成且最终收敛为**单一 `send_key(ctx, is_keydown)`**（无 `emit_output`/`EmitMode`/`emit_tap` 残留）；记录统一在函数内 `record_emitted`；单一「按住→release」模型覆盖所有单键输出。resolver 合并（抽 `resolve_output` 合并 `phase7_resolve` / `phase_up4_resolve`）待实施。

**send_key 去掉 is_keydown：「怎么发」与「需不需要发」彻底分离（2026-07-10 01:4x，已实施 ✅）**
- 用户复盘：「还是差点，别用 is_keydown 这个值。层键功能不发送的情况在上层过滤掉。send_key 只负责判断怎么发，不判断需不需要发。」
- 落地（`pipeline.rs`）：
  - `send_key(&mut self, ctx: &mut Context)`：**删除 `is_keydown` 参数**，`send_key` 只决定「怎么发」——
    - 层键 → **始终** `activate_layer` + 记 `target_layer`（不再有 keydown/keyup 分支，因为层键仅 keydown 路径经上层放行到达此处）；
    - 单键 → `emit_down` + 记 `emitted`；文本·宏·鼠标 → `emit_text/run/mouse_move` + 记 `emitted`；`{Sleep N}` 前缀递归。
    - 同时**删除内部空输出 `return`**（原本的「需不需要发」判断）：「需不需要发」移到上层。
  - 「需不需要发」上移到 phase 层过滤：
    - `phase_up5_output`（keyup）：**若 `is_layer_key(logical_key)` 直接 return**（层键 keyup 不发送，反激活交给 `release_key` 读 `target_layer`）；空输出也直接 return。
    - `phase8_final`（keydown）：空输出已在此过滤（保留原 `if logical_key.is_empty() { flags|=F_FINAL; return }`）。
  - 这样 send_key 成为纯「按类型选发送方式 + 记录」的函数，层键 keyup 的「不发送」决策由上层 phase 守卫，职责单一清晰。
- 验证：`cargo test` 全绿（39 项），无 warning。

**resolver 合并（抽公共 resolve_output，2026-07-10 01:5x，已实施 ✅）**
- 用户复盘：「合并之后 resolver 应该具有解算映射所有状态的功能」——把 `phase7_resolve`（keydown）与 `phase_up4_resolve`（keyup）里重复的 `td_action → logical_key` 映射抽成公共 `resolve_output(ctx)`；phase 层只保留 flags/debug + combo 优先逻辑，核心调 `resolve_output`。
- 落地（`pipeline.rs`）：
  - 新增 `resolve_output(&mut self, ctx: &mut Context)`：合并前两份各自重复的分支（Intercept / Tap / Hold / Holding / TwoTap / DoubleTap / Custom / HoldRelease / DoubleHoldRelease + 默认 tap），**单一 match 覆盖全部 9 个非 None 变体**，穷尽性由末尾 `_ => {}`（仅兜底 None，已被外层 `if td_action != None` 排除）满足。
  - 各 td_action 只在其所属方向的管道被设置（keydown 不会出现 HoldRelease；keyup 不会出现 Intercept/Hold/Holding/Custom），合并为单一 match 安全——未触发的分支为死代码但零行为差异；`HoldRelease`/`DoubleHoldRelease` 的 emitted→base.hold→resolve 回落逻辑从 keyup 原样并入。
  - `phase7_resolve`（keydown）：保留 combo 优先块（仅此方向有），其后 `self.resolve_output(ctx)` + `ctx.flags |= F_RESOLVE;`（原 `if self.keys.contains_key {...}` 大段删除）。
  - `phase_up4_resolve`（keyup）：仅 `self.resolve_output(ctx)`（无 combo 优先——combo 释放由 `release_key` 按 pending_up / combo.output 处理，原 F_UP4 末尾不置位行为保留）；原整个 `td_key = ctx.key.clone()` + match 大段删除。
- 验证：`cargo test` 全绿（pipeline 26 / doubletap 6 / reachability 1 / scenario 1 / main 5，共 39 项），无 warning。

**状态**：input（resolver）与 output（send_key）两侧「keydown/keyup 公共函数」合并均已完成——`resolve_output` 合并 phase7/phase4，`send_key` 合并 phase8/phase5（无 emit_output/EmitMode/emit_tap 残留）。管道结构保留，phase 层只做 flags/debug/combo 优先等门控，核心调公共函数。keyup 的 combo 释放仍由 release_key 处理（未并入 resolve_output，符合既有设计）。

### 三、待办（回来后）

- [x] 抽公共 `emit_output(ctx)` 合并 `phase8_final` 与 `phase_up5_output`（✅ 已落地 2026-07-09 22:5x）。
- [x] emit_output 「统一记 emitted」实验 → 引双发 Up bug（test_hold_only_doubletap），**已按 fallback 改为纯发送、记录上移至 phase8_final**（2026-07-09 23:1x，28 测试仍绿）。
- [x] 落实公共 `resolve_output(ctx)` 合并 `phase7_resolve` 与 `phase_up4_resolve`；删重复映射（根除分叉 bug 土壤）（✅ 已落地 2026-07-10 01:5x，39 测试全绿）。
- [ ] 合并后与 `leader-plan.md` Step0 协同评审（输出口唯一化是否因此自动达成，phase_leader 插入点是否简化）。
- [x] 全局回归：golden scenario + 现有 39 测试须全绿（已满足）。

---

## 历史节点（旧）

- 2026-07-09 下午：`reenter_up` 前置的 keyup 阶段拆分、flags 取代 `needs_output`、`release_only` 清理。
- 2026-07-09 早些：combo 抬起发送移入 final（`pending_up` 分层）。
- 计时器懒删「复活旧计时器」修复（双击 hold-only 误输出 12）。
- RUN: 宏带参 / 中文空格路径修复。
- Leader 设计讨论（见 `leader-plan.md` v2.3）。
