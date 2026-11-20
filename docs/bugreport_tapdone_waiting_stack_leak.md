# 报告：TapDone 结算未出栈导致 waiting_stack 残留

> 状态：**已实施（2026-11-20），待实机验证**
> 关联现象：Revit 中单击 space 后，右键→中键映射从「可按住」退化为「只能点击」；双击 space 后恢复。
> 分析基线：`feat/portable-backend`，`anykey-engine/src/{pipeline.rs, state.rs, pipeline/tap_dance.rs, pipeline/defer.rs}`
> 实施记录见文末 §9。

---

## 1. 结论摘要

右键映射本身没问题。真正的卡点是 `state.defer.waiting_stack` 里残留了一条 `("space", SwitchKind::Hold)`：

- 单击 space → `TdKind::TapDone` → `dt_timer` 到期 → `flush_tapdone_as_tap()` → `reenter_up()`；
- 这条结算路径**只清 td/emitted/target_layer，从不出 waiting_stack**；
- 残留条目让 `try_defer_layer_interrupt()` 把后续所有「当前层无 TD 定义」的键（含 mouseright）塞进 `keys_deferred`（日志里的 `wsLen=1` → `[SKIP] PHASE5-TD` → `[SKIP] PHASE7-Resolve`）。注意：此时 space 的 td 已被 up8 清成 None，但 `defer.rs:92` 只判 `waiting_stack` 是否为空；`target_anchor_layer()` 找不到 Waiting 层键时回落 `current_layer()` 并返回 `Some(...)`，不会触发 `return false`，所以幽灵条目照样能让延迟成立；
- 右键的 Down 被推迟到 keyup 时的 `phase_up2_flush`，Down/Up 挤在同一周期 → 表现为「只能点击」；
- 双击 space 命中 `TapDone → DoubleTapHold`，经 `ctx.force_hold_down` 走 P6 `force_hold_execute`，该函数会移除 trigger 自己 → 栈清空 → 右键恢复。这正好解释了用户的「自救」操作。

**推荐落点**：在 `flush_tapdone_as_tap()` 内部、调用 `reenter_up()` 之前登记 `ctx.force_hold_up = Some(key.to_string())`，由既有的 up4 → `force_hold_execute()` 统一出栈。不要在 `dt_timer` / `td_interrupt_all` 里各塞一行 `remove_waiting_stack_item`。

---

## 2. waiting_stack 的完整生命周期

### 2.1 入栈点（仅 2 处，都在 `tap_dance.rs`）

| 位置 | 条件 | 标签 |
|---|---|---|
| `tap_dance_down()` :103 | 首次按下，`is_switch_key(key)`（hold 字段是层键/修饰键） | `SwitchKind::Hold` |
| `tap_dance_down()` :84 | 第二次按下确认 `has_dh` 且 `is_double_switch_key(key)` | `SwitchKind::DoubleHold`（已有 Hold 则升级） |

入栈的语义是「这个键的 hold/double_hold 还没结算，后续键需要为它让路」。

### 2.2 出栈点（共 4 处）

| # | 位置 | 所属机制 | 出栈时机 |
|---|---|---|---|
| 1 | `defer.rs:33` `force_hold_execute()` | keydown P6 / keyup up4 的统一 ForceHold 出口 | 激活 trigger **之前**的所有 waiting 层键后，移除 trigger 自己；trigger 不在栈中（普通键/哨兵）则先全激活整栈，remove 变 no-op |
| 2 | `defer.rs:76` `force_hold_switch_key()` | 单个层键被真正激活 | `fire_switch()` 之后立即移除该 td_key，并置 Holding / DoubleHolding |
| 3 | `tap_dance.rs:290` `hold_timer()` | Hold 计时器到期（TD 结算） | `re_enter` → 设 Holding → flush 挂在自己背后的 `keys_deferred` → **步骤 4 出栈** |
| 4 | `tap_dance.rs:330` `dh_timer()` | DoubleHold 计时器到期（TD 结算） | `re_enter` → flush 背后 `keys_deferred` → **末尾出栈** |

### 2.3 keyup 侧的结构约定：出栈不写在业务分支里

真实 keyup 的 tap 结算**从不直接调用 `remove_waiting_stack_item`**，而是一律登记 trigger，交给 up4 统一处理：

| 登记点 | 写入字段 |
|---|---|
| `tap_dance_up()` :185（Waiting/None 立即 tap） | `ctx.force_hold_up = Some(key)` |
| `tap_dance_up()` :205 / :219（TapDone / DoubleWait 确认） | `ctx.force_hold_up = Some(key)` |
| `phase_up1_combo()` :343（combo Seeking 回退 tap） | `ctx.force_hold_up = Some(key)` |
| `phase_up2_flush()` :376 / :382（延迟源键 Waiting / DoubleWait 抬起） | `ctx.force_hold_up = Some(ctx.key)` |

`pipeline.rs:410-411` 的注释把这条约定写得很明确：

> keyup 侧分支（phase_up2_flush 延迟源头键 Waiting / phase_up1_combo Seeking / tap_dance_up Waiting）
> 写入触发键，统一在此处经 force_hold_execute 激活+移除自己，彻底收敛到这条对称通道。

也就是说：**「激活 trigger 之前的层键 + 移除 trigger 自己」是一个不可拆开的原子语义，只由 `force_hold_execute` 承担。** 这正是判断「出栈该放哪」的准绳。

---

## 3. TapDone 结算路径为什么漏了

### 3.1 `flush_tapdone_as_tap()` 是 TapDone 结算的统一封装

`tap_dance.rs:14-20`：

```rust
fn flush_tapdone_as_tap(&mut self, key: &str) {
    let mut ctx = Context::new(key);
    ctx.flags = REENTER_UP_SKIP;
    self.reenter_up(&mut ctx, REENTER_UP_SKIP);
}
```

函数注释声明它「统一封装 … 供 dt_timer（定时器到期）、td_interrupt_all（被第三键打断）、phase_up2_flush（延迟键释放时源头键已 TapDone）**三条路径共用**，避免各处手写时漏掉 reenter_up」。

实际调用点只有 **2 处**：
- `dt_timer()` :302
- `td_interrupt_all()` 的 `Some(TdKind::TapDone)` 分支 :357

（第三处已不存在，注释过时。）

### 3.2 关键：注释承诺的「TapDone 分支自己登记 force_hold_up」没有执行到

`tap_dance.rs:160-162` 写着：

> force_hold_up 只在「无 dt/dh」的立即 tap 路径内登记：dt/dh 路径走 TapDone 由 dt_timer 到期经 reenter_up 重新进入，**届时由 TapDone/DoubleWait 分支自身登记 force_hold_up**。

但 `REENTER_UP_SKIP`（`state.rs:95`）定义为：

```rust
F_UPCOMBO | F_UPFLUSH | F_UPTAPDANCE | F_UPFLUSHBEHIND
```

其中 `F_UPTAPDANCE` 会让 `phase_up3_td()` 直接返回 → **`tap_dance_up()` 里的 TapDone/DoubleWait 分支根本不会运行**，自然也不会登记 `force_hold_up`。

于是 `flush_tapdone_as_tap()` 的 `reenter_up()` 实际跑的是：

| 相位 | 是否执行 | 做了什么 |
|---|---|---|
| up1 combo | 跳过 | — |
| up2 flush | 跳过 | — |
| up3 td | 跳过 | ← 本该登记 force_hold_up 的地方 |
| up4 forcehold | 执行 | `ctx.force_hold_up == None` → no-op，**不出栈** |
| up5 commit | 执行 | 解析 tap 输出、emit Down |
| up6 release | 执行 | emit Up |
| up7 flushbehind | 跳过 | ← 背后延迟键也不会被 flush |
| up8 clean | 执行 | 清 emitted / target_layer / td / combo，**不碰 waiting_stack** |

### 3.3 与其他两条 TD 结算路径对比

| 结算路径 | re_enter 通道 | 出栈 | flush 背后延迟键 |
|---|---|---|---|
| `hold_timer`（Waiting → Holding） | `re_enter(COMMIT_SEND_REENTER)` | ✅ :290 | ✅ :285-287 |
| `dh_timer`（DoubleWait → DoubleHolding） | `re_enter(FLUSH_DEFERRED_REENTER)` | ✅ :330 | ✅ :325-327 |
| `flush_tapdone_as_tap`（TapDone → Finished） | `reenter_up(REENTER_UP_SKIP)` | ❌ **缺口** | ❌（up7 被跳过） |

`flush_tapdone_as_tap` 是三条 TD 结算路径中唯一没有出栈步骤的，这就是本次 bug 的结构性根因。

---

## 4. 推荐落点与具体改法

### 4.1 落点

**`anykey-engine/src/pipeline/tap_dance.rs`，`flush_tapdone_as_tap()` 内，`reenter_up()` 之前。**

### 4.2 改法（示意）

```rust
fn flush_tapdone_as_tap(&mut self, key: &str) {
    let mut ctx = Context::new(key);
    ctx.flags = REENTER_UP_SKIP;
    // TapDone 的物理键已抬起，不会再有真实 keyup 走 up3 登记 force_hold_up
    // （REENTER_UP_SKIP 跳过 up3）。与 tap_dance_up 的 TapDone 分支对称，
    // 在此登记触发键，交 up4 → force_hold_execute 激活其前的 waiting 层键并出栈。
    ctx.force_hold_up = Some(key.to_string());
    self.reenter_up(&mut ctx, REENTER_UP_SKIP);
}
```

一个修改点同时覆盖 `dt_timer` 与 `td_interrupt_all` 两条调用路径。

### 4.3 为什么这是「符合现有结构」的位置

1. **复用唯一出栈契约**：仍然经 `ctx.force_hold_up → phase_up4_forcehold → force_hold_execute`，没有新增第二套出栈机制。
2. **补上被跳过的设计意图**：`tap_dance.rs:160-162` 明确承诺 TapDone 结算要登记 force_hold_up，只是执行点因 up3 被跳过而落空；这里正是那个缺失的执行点。
3. **落在统一封装内部**：函数存在的理由就是「避免各处手写漏步骤」，出栈与 `reenter_up` 一样属于「每个调用者都不该各自记得做」的步骤。
4. **与真实 keyup 对称**：`tap_dance_up()` 的 TapDone 分支（:205）在真实第二次抬起时同样登记 `force_hold_up`；定时器结算走同一条路，语义一致。
5. **语义对两类键都正确**：
   - switch 键（如 space，在栈中）→ `force_hold_execute` 走 before-self，激活它之前的层键并移除自己；
   - 非 switch 键（不在栈中）→ 走「全激活整栈」分支，与 `tap_dance_up` :204 注释里「本键通常非 switch 键 → 全激活整栈 waiting」的既有设计一致。
6. **不改变现有输出**：本次场景 waiting_stack 只有 `[space]`，before-self 子集为空，只是把 space 移除；up5 的层解析、up6 的发送、up8 的清理都不受影响。

---

## 5. 为什么不选其他位置

| 候选 | 判定 | 理由 |
|---|---|---|
| `dt_timer()` 和 `td_interrupt_all()` 各加一行 `remove_waiting_stack_item` | ❌ | 绕过 force-hold 的 before-self 层激活语义；两处手写正是封装函数要消除的漏项模式；日后第三个调用者仍会漏 |
| `flush_tapdone_as_tap()` 内直接调 `remove_waiting_stack_item` | ⚠️ 能止血但不推荐 | 只修「栈残留」表象，丢失「激活 trigger 之前层键」的配套语义；与 keyup 侧其他路径的出栈通道不一致 |
| `reenter_up()` 里统一出栈 | ❌ | `fire_sleep_timer()`（`pipeline.rs:568`）等也复用 `reenter_up`，它们的 key 与本轮 waiting_stack 无关，会误清栈 |
| `phase_up8_clean()` | ❌ | 职责是「本键 emitted / target_layer / td / combo 残留清理」，语义不含 waiting_stack；更致命的是真实 keyup 的 Waiting → TapDone 路径**必须保留**栈条目供第二次按下判定 double tap，在那里出栈会直接毁掉双击 |
| `try_defer_layer_interrupt()` 里判断 td 状态后跳过 | ❌ | 治标：只让这个键不被延迟，栈里的幽灵条目仍在，会继续污染层锚点判定与 combo 查表 |

### 5.1 专项：把 `REENTER_UP_SKIP` 放开、让 up3 参与，是否可行？

**不可行。** 逐条核实如下。

**(a) `dt_timer` 路径：tap 会消失，而且 bug 仍然没修。**

`dt_timer` 在调用 flush **之前**已把 td 设成 Finished（`tap_dance.rs:301`）：

```rust
self.set_td_state(key, TdKind::Finished);
self.flush_tapdone_as_tap(key);
```

若 up3 不再跳过，`tap_dance_up()` 会命中 `Some(TdKind::Finished)` 分支（`:194-200`）：

```rust
ctx.td_action = TdAction::Intercept;
ctx.flags &= !(F_UPCOMMIT);
```

而 resolver 对 `Intercept` 的处理是 `ctx.logical_key.clear()`（`commit.rs:87`）→ **本该补发的单 tap 变成无输出**。日志里 dt_timer 现在能发出 `EmitDown: space`，靠的正是 `td_action=None` 时 `Context::new` 保留的 `{space}` 默认值。

更关键的是：Finished 分支**不写 `force_hold_up`**，所以 up4 依然 no-op —— 开着 up3 也修不掉出栈问题。

**(b) `td_interrupt_all` 路径：语义反转，会发成 doubleTap。**

该路径调用 flush 时 td **仍是 TapDone** —— `set_td_state(k, Finished)` 在 flush **之后**（`tap_dance.rs:357-358`）：

```rust
self.flush_tapdone_as_tap(k);
self.set_td_state(k, TdKind::Finished);
```

若 up3 不再跳过，`tap_dance_up()` 会命中 `Some(TdKind::TapDone)` 分支（`:202-216`），按 `double_tap` 字段把 `td_action` 设为 `DoubleTap` 或 `TwoTap`。但 flush 的语义是「双击窗口已超时／被第三键打断 → 补发**单 tap**」，与「双击确认」恰好相反。用户的 space 配置里 `fn2 tap=''`、`dt='dbl{space}tap'`，两者输出完全不同 —— 会打出错误内容。

**(c) up3 内的 `td_interrupt_all` 会重入递归。**

`phase_up3_td()` 每次都先执行 `self.td_interrupt_all(&key, true)`（`pipeline.rs:403`）。形成环：

```
td_interrupt_all(A) → flush(B) → reenter_up 跑 up3 → td_interrupt_all(B)
                    → 若 A 仍是 TapDone → flush(A) → up3 → td_interrupt_all(A) → …
```

同时还会二次打断其他 Waiting 键；参数 `true`（`force_hold_before`）又会在 keyup 侧额外激活前置层键，偏离 up3 原有的 keyup 约定。

**(d) 设计层面的判断。**

`REENTER_UP_SKIP` 跳过 up3 不是遗漏，而是把 flush 定义为「人造的、强制结算为单 tap 的 keyup」；真实 keyup 的 TD 状态机里 `TapDone` 却代表「双击确认」。让 up3 参与，等于让「超时兜底」走「确认双击」的分支。

`tap_dance.rs:160-162` 那条「届时由 TapDone/DoubleWait 分支自身登记 force_hold_up」的注释看起来是历史遗留：它假设 reenter 从 up3 开始，而现行 `REENTER_UP_SKIP` 明确跳过 up3。**注释过时，不等于漏了一个标志。**

若坚持让 up3 参与，必须同时：① 改 `dt_timer` 的状态设置顺序；② 让 flush 能区分「超时兜底」与「双击确认」两种语义；③ 给 `td_interrupt_all` 加重入保护。三处改动，复杂度远高于在 flush 内加一行 `force_hold_up`。

---

## 6. 附带发现的两个缺口（建议一并评估，不属本次最小修复）

1. **背后延迟键不会随 TapDone 结算发出。** `hold_timer` / `dh_timer` 都显式 flush 了 `keys_deferred` 中 `td_key == key` 的条目，而 `flush_tapdone_as_tap` 依赖的 up7_flushbehind 被 `REENTER_UP_SKIP` 跳过。即使按第 4 节补了出栈，这个问题仍然存在。若需要对称，应同样在 `flush_tapdone_as_tap` 内补一次 behind-flush（注意放在 `reenter_up` 之前还是之后，要照 `hold_timer` 的「key 仍在栈中时 flush，用 key 定位边界」顺序处理）。

2. **文档注释过时（已随本次修改同步修正）。** `flush_tapdone_as_tap` 头部注释原先说「三条路径共用」，实际只有 2 个调用点；`tap_dance.rs` 里「届时由 TapDone/DoubleWait 分支自身登记 force_hold_up」的承诺因 up3 被跳过而从未兑现。两处均已改为与现行代码一致的表述。

---

## 7. 回归测试建议

按项目惯例写成独立集成测试文件（`anykey-engine/tests/`），不要塞回内联测试。

**测试 A：dt_timer 结算后栈必须清空**

- 配置：`space` = `tap={space}, hold={fn1}, ht=200, dtt=250, dt={space}`（与用户同构）；另配一个无 TD 的 `mouseright` = `tap={mousemiddle}`。
- 步骤：`key_down("space")` → 断言 `waiting_stack` 含 space → `key_up("space")` → 断言 td=TapDone（此时仍在栈，符合双击预留）→ `advance_ms(250)` 触发 dt_timer。
- 断言：
  - `td("space").is_none()`（现有 `doubletap_hold_test.rs:215` 已有类似断言）；
  - **`waiting_stack` 不含 space**（本次新增，修复前必失败）；
  - 随后 `key_down("mouseright")`：`keys_deferred` 不含 mouseright，且 emit_log 在 **DN** 时点就有 `Down(mousemiddle)`（修复前只会在 UP 时点出现）。

**测试 B：第三键打断 TapDone 时也要出栈**

- 复用 `pipeline_test.rs:428-445` 的场景（`space` TapDone，被 `z` 的 keyup 打断），在 `s.key_up("z")` 后追加断言 `waiting_stack` 不含 space。这条覆盖 `td_interrupt_all` → `flush_tapdone_as_tap` 入口。

**测试 C（防回归）**：单击 space 后 250ms 内的第二次按下仍应正确进入 `DoubleTapHold`，确认出栈没有提前发生、也没有破坏双击判定。

---

## 8. 实施前置条件

按项目约定，动手前应先备份当前工作区到 `backups/<日期>_tapdone_waiting_stack/`，并且：

- 当前分支 `feat/portable-backend` 有未提交改动：`CHANGELOG.md`、`anykey-engine/src/main.rs`、`tools/`（未跟踪）。备份时一并保留，不要回滚。
- 编译/测试必须带 `CARGO_TARGET_DIR=C:/Users/proje/AppData/Local/Temp/<name>`，避免工作区内链接死锁。
- 文档如需入库，走 `tools/doc_to_main.py`，不要在本分支直接提交 `docs/`。

---

## 9. 实施记录（2026-11-20）

**备份**：`backups/20261120_tapdone_waiting_stack/`
含实施前的 `CHANGELOG.md`、`main.rs`、`tap_dance.rs`、本报告、`tools/`（159 项），以及实施后的 `after_fix_tap_dance.rs`。原有未提交改动均未回滚。

**代码改动**（`anykey-engine/src/pipeline/tap_dance.rs`，13 增 7 删）：

1. `flush_tapdone_as_tap()` 在 `reenter_up()` 前新增 `ctx.force_hold_up = Some(key.to_string())` —— 本次功能修复的唯一一行。
2. 同步修正该函数头部注释（两条调用路径、职责含出栈）与 `tap_dance_up` 中关于 force_hold_up 登记时机的过时注释。

**新增测试**：`anykey-engine/tests/tapdone_waiting_stack_test.rs`，4 项：

| 测试 | 覆盖 |
|---|---|
| `test_dt_timer_settlement_pops_waiting_stack` | dt_timer 结算后 td 清空、space 出栈、补发单 tap 的 Down+Up |
| `test_after_dt_settlement_mouse_right_is_not_deferred` | 栈空后右键 DN 立即发 `MouseDown(mousemiddle)`，不再被 defer |
| `test_td_interrupt_all_tapdone_also_pops_stack` | 第三键打断 TapDone 这条调用入口同样出栈 |
| `test_double_tap_still_works_after_fix` | 250ms 内二次按下仍确认 `DoubleTapHold`、发 dt 输出 |

**验证结果**：

- 对照实验：临时移除那一行后跑新测试 → **3 失败 / 1 通过**；恢复后 → **4 全通过**。确认测试确实捕捉原 bug。
- 全量 `cargo test`（`CARGO_TARGET_DIR=C:/Users/proje/AppData/Local/Temp/anykey_tapdone_test`）：**150 项全部通过，0 失败**（基线 146 + 新增 4）。
- 过程中的两个测试写法坑（已修正）：① `advance_ms(250)` 恰好没跨过 `deadline_tick=250`，必须用 260；② 配置层名是 `baseLayer` 而非 `base`，用错会凭空多出一层。

**尚未做**：实机验证（需用户在 Revit 中复现「单击 space 后右键可按住」）；§6 的第 1 个缺口（背后延迟键 flush 不对称）未纳入本次修复。
