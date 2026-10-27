# 覆盖层的「删除」语义：`{unmap}` 标记

> 一句话：给应用/设备覆盖加一种「在本层把这条映射去掉」的写法 —— 把条目的输出（或 TapDance 的某个字段）写成 `{unmap}`。
>
> **状态：已实施（2026-10-27，引擎侧）。** 配置格式与 GUI 均未改动；施工记录与实现中发现的差异见 §11。

---

## 1. 问题：覆盖桶只能加，不能减

**用户场景**：玩街霸 6 时打开了 AnyKey，操作出现延迟、部分 combo 误触。期望是「运行这个应用时，指定按键完全不参与任何重映射」，回到原生手感；退出游戏后全局配置照常生效。

**现状缺口**：应用覆盖（`appAware.apps[app]`）、设备覆盖（`devices[guid]`）、设备+应用覆盖（`devices[guid].apps[app]`）三个桶**只能新增或改写条目，没有任何「删除」的表达**：

- 引擎侧三个合并函数只做 `insert`，没有删除路径 —— `anykey-engine/src/runtime_builder.rs:68-98`（`merge_combos` / `merge_leader` / `merge_layers`）。
- GUI 侧的写回是**反向 diff**：只把「视图里有、且与基线不同」的条目写进覆盖桶 —— `gui/main.py:4163-4207`。因此**在应用/设备视图里删掉一条全局条目，视图里没有它、diff 没有输出，下次重载又从全局合并回来** —— 静默无效。只有删「本层自己写过的覆盖」才有效果。

所以缺的不是 UI 控件，而是**配置层面的否定语义**。

---

## 2. 候选方案与取舍

| 方案 | 结论与理由 |
|------|-----------|
| 每个覆盖桶加独立 `cleared` 段（三个数组） | 否决。要新增结构体、新增一套 identity 序列化格式；GUI 必须理解「删除语义」，还得额外做「已清除清单 + 恢复」UI；最大风险是**把视图 bug 推导成 tombstone**，静默失效且无法自查 |
| 行上加 `disabled: true` 标记 | 否决。要把整行内容抄进覆盖桶，全局改了之后就是陈旧副本 |
| 覆盖改成整表替换（replace 模式） | 否决。全有或全无，失去继承；一个应用只想清 2 条却得复制 40 条 |
| **输出值 `{unmap}`** | **采纳** |

采纳理由：

1. **零结构变更** —— 不动 config schema，不引入新类型；旧引擎读新配置、新引擎读旧配置都不会崩。
2. **零新 UI** —— combo/leader 表格本来就能填输出，TapDance 编辑面板本来就能填 `tap`/`hold`/`dt`/`dh`。
3. **可见即可恢复** —— 被清除的条目就明摆在表里那一行（输出列是 `{unmap}`），改成具体值就恢复，不需要任何清单界面。
4. **它把「删除」从推导出来的隐式语义，变回用户显式写下的值** —— 消掉了方案一最大的风险面。
5. 导入/导出页面、手写配置、第三方工具都天然带上它。

---

## 3. 语法与定位

- **精确字面 `{unmap}`**，大小写不敏感（与 `{Sleep N}`、`{Select N}` 的既有约定一致）。
- 它是**构建期标记，不是运行时指令**：引擎在 `build_mapping()` 里解释掉它，之后才构造 `tap_dance` / `combo_index` / `LeaderDef`；管道（`resolver` / `send_key`）永远看不到这个字符串。
- 与 `{fn1}` / `{Sleep 300}` 的关系只是**借了同一种写法**（从而复用现有输入框与 `normalize_key_output`），语义上不属于「输出指令」族。它是配置元数据。
- **裸写 `unmap`（不带花括号）不认**：`normalize_key_output` 不会给它加括号（多字符、非已知键名），引擎会把它当文本原样打出来。结果是可见的、不静默 —— 这也是不做 GUI 校验的前提。

---

## 4. 语义

| 条目类型 | `{unmap}` 的效果 |
|---------|-----------------|
| combo 行（`output`） | 该行**不成立**，从本层消失；相关按键不再进 `combo_index` → 不进入 `Seeking`、没有 `comboTime` 等待 |
| leader 序列（`output`） | 该序列**不成立** |
| TapDance `tap` / `hold` / `dt` / `dh` 单个字段 | **该字段取空值**，本行其余字段保留自己的值 |
| TapDance 四字段全空 | 该层**整条不存在**（`ht`/`dtt`/`dht` 一并失效，不再进 `tap_dance`）→ 按键**原生直通** |

### 4.1 「整条空」的判定基准

现有合并是**整条替换**，不是逐字段合并 —— `merge_layers` 里是 `target.insert(phys.clone(), entry.clone())`（`runtime_builder.rs:87-98`）。据此把规则钉成：

> `{unmap}` = **该字段取空值**；当本层**最终生效的那条 KeyEntry** 的 `tap`/`hold`/`dt`/`dh` 全为空时，该层没有这个条目。

两点必须明确：

- 看的是**本层最终值**，不是「相对下层做字段级 diff」。GUI 写覆盖时本来就写整行（`_diff_layers` 输出的是视图里那整条 entry），所以实际使用中两种解释结果一致；手写的部分行 JSON 则遵守「本层没写就是没有」，行为可预测。
- 若日后要改成「未提及字段继承下层」，那是**另一次语义变更**（会改变现有配置的行为），不要混进这个特性。

### 4.2 逐字段示例

```json
// 全局：CapsLock 点按发 Esc，按住切 fn1 层
"tapDance": { "base": {
  "capslock": { "tap": "{esc}", "hold": "{fn1}", "ht": "150" }
} }

// 应用覆盖行：只想去掉「按住切层」，点按保留 Esc
"tapDance": { "base": {
  "capslock": { "tap": "{esc}", "hold": "{unmap}" }
} }
→ 生效：tap={esc}，hold 空 → 纯点按重映射，无 hold 计时延迟
```

```json
// 应用覆盖行：四个字段都清 → 整个条目消失 → CapsLock 完全不受管
"tapDance": { "base": {
  "capslock": { "tap": "{unmap}", "hold": "{unmap}", "dt": "{unmap}", "dh": "{unmap}" }
} }
```

```json
// 应用覆盖行：清掉一条全局 combo
"comboMap": [ { "key1": "a", "key2": "s", "output": "{unmap}", "layer": "base" } ]
```

```json
// 应用覆盖行：清掉一条全局 leader 序列
"leader": [ { "keys": ["q", "w"], "output": "{unmap}" } ]
```

---

## 5. 实现落点：引擎 3 个文件，GUI 0 行

### 5.1 为什么可以只在「构建前」解释一次

因为合并是整条替换（§4.1），`{unmap}` 会**随最后写入的那一行**一路走到最终切片。所以在构造索引之前扫一次就够，三个 `merge_*` 函数**一行都不用改**，`apply_app_override` 的四层调用点也不用改。

> **这条结论有前提**：合并必须保持「整条替换」。若哪天真把 `merge_layers` 改成字段级合并，单点解释的假设要重新审视。

### 5.2 改动清单

| 文件 | 改动 |
|------|------|
| `anykey-engine/src/util.rs` | `pub const UNMAP_TOKEN: &str = "{unmap}"` + `pub fn is_unmap_output(s: &str) -> bool`（trim + `eq_ignore_ascii_case`） |
| `anykey-engine/src/runtime_builder.rs` | 新增 `build_mapping(layers, combos, leader) -> DeviceMapping`：先 `apply_unmap`，再建三个索引。**所有构建点改为走它**，顺带消掉现在 4 处重复的「三件套」构造 |
| `anykey-engine/src/config.rs` | `collect_unmap_warnings(&Config) -> Vec<String>`：只找**全局层**的 `{unmap}`（`comboMap` / `tapDance` / `leader.sequences`），返回可读文案 |
| `anykey-engine/src/main.rs` | 配置加载后逐条打印上述告警 |

`apply_unmap` 骨架（~15 行）：

```rust
fn apply_unmap(layers: &mut Layers, combos: &mut Vec<ComboRow>, leader: &mut Vec<LeaderSequence>) {
    combos.retain(|r| !is_unmap_output(&r.output));
    leader.retain(|s| !is_unmap_output(&s.output));
    for map in layers.layer_maps.values_mut() {
        for e in map.values_mut() {
            for f in [&mut e.tap, &mut e.hold, &mut e.dt, &mut e.dh] {
                if is_unmap_output(f) { f.clear(); }
            }
        }
        map.retain(|_, e| !(e.tap.is_empty() && e.hold.is_empty()
                          && e.dt.is_empty() && e.dh.is_empty()));
    }
}
```

需要改走的构建点（当前都是手写三件套 `build_tap_dance_map` + `build_combo_index` + `LeaderDef{}`）：

1. `build_default_device_context`（单设备/全局路径）
2. `build_multi_device_contexts` 的**全局预构建**（`global_td` / `global_combo` / `global_leader`，合成一个 `global_mapping` 供未订阅、无覆盖的设备 clone）
3. `build_multi_device_contexts` 的**覆盖分支**（合并后的切片）
4. `apply_app_override` 的收尾（四层链最终切片）

`build_default_runtime`（测试用兼容包装）随 `build_default_device_context` 自动保持一致。

### 5.3 生效时机

沿用现有规则：GUI 只写 config，托盘「重载设置」重启引擎后生效。不需要新增 IPC。

---

## 6. GUI 为什么零改动

三条链路天然正确（逐条核对过）：

1. **归一化不改写它** —— `normalize_key_output("{unmap}")`：`_is_wrapped_key_output` 命中，内部无空格无嵌套花括号，查别名表兜底返回原值，得到仍是 `{unmap}`，不会变成 `{{unmap}}` 也不会被包成裸文本（`lib/config.py:203-222`）。
2. **视图里照常显示** —— `_resolve_app_view` 合并时，覆盖行的 `{unmap}` 正常覆盖全局那一行（`gui/main.py:4209-4238`）。用户在原位就能看见「这行被设成 `{unmap}` 了」，改回具体值即恢复。
3. **diff 把它当普通值写回** —— `_diff_combos` / `_diff_layers` / `_diff_leader`（`gui/main.py:4163-4207`）与 `_flush_active_mapping`（`:4287-4375`）不需要任何特判；删掉那一行就是撤销覆盖、恢复继承。
4. 另外 `_collect_combo_rows` 只要求 `output` 非空（`:1764-1788`），`{unmap}` 可以直接过关。

所以「显示 / 编辑 / 恢复 / 导出导入」全部免费继承，不需要新控件、新弹窗、新清单。

---

## 7. 边界与已知取舍

| 事项 | 结论 |
|------|------|
| **只在覆盖链里「删除继承值」** | 全局层（`comboMap` 顶层 / `tapDance` / `leader.sequences`）写 `{unmap}` = 该字段按空处理 / 该行不成立，并在启动时**逐条告警**「全局层不支持 {unmap}，已按不存在处理」。没有下层可删，写在这里只是写错了位置 |
| 裸写 `unmap`（无花括号） | 引擎不认，被当文本输出 → 真打出 `unmap` 五个字母。可见、不静默（因此不需要 GUI 校验） |
| 只清 TD、没清同名 combo | 该键仍会因 combo 的 `key_set` 被 `build_tap_dance_map` 拉进 `tap_dance`（identity 兜底，`tap` 默认 `{自身}`）。行为等于原生，但仍过一次管道 → **要彻底「无感」需把相关 combo 一起清掉** |
| `perDevice=false` | 设备层覆盖本就被引擎忽略（`apply_app_override` 用 `device_id=0` 查不到设备桶），`{unmap}` 同理只在应用层生效 —— 与现有行为一致 |
| 驱动 / llhook 两种后端 | 无差异。两个后端都是「先吞后重发」，`{unmap}` 的作用是让键在映射表里不存在 → 引擎按自身身份原样重发 |
| 旧引擎读新配置 | 把该行输出当未知键名 → `UNKNOWN KEY`（`pipeline/commit.rs:447`），**只发 debug 日志、什么都不发**。因此不建议新旧版本混跑，也不建议把 `{unmap}` 用在长期共享的配置里 |
| 性能 | `{unmap}` 只在构建期处理（应用切换、托盘重载），热路径零影响 |
| **明确不做** ① 启动时统计 clear 行数的日志 | 已有 `dump_mapping` 在 debug 下打印完整映射，统计日志是冗余 |
| **明确不做** ② GUI 保存期校验 / 提示 | 用户定调：不做。代价是裸写 `unmap` 会打出文本，属可见错误 |
| **明确不做** ③ 行内灰底 +「已清除」标签 | 值本身已经自解释，先不加视觉修饰 |
| **明确不做** ④ 「整键清空」批量按钮 | 纯 GUI 便利（帮你批量填 `{unmap}`），与引擎无关，留作后续可选项 |

---

## 8. 测试清单

引擎集成测试（`anykey-engine/tests/runtime_builder_test.rs` 为主，管道级放 `pipeline_test.rs`）：

1. **combo**：应用层 `{unmap}` → 该 combo 从最终 `combo_index` 消失；管道级断言按住/单按该键立即输出自身、`pop_expired_timers` 无 `ComboTimeout`。
2. **TD 单字段**：`hold={unmap}` → `tap` 保留、`hold` 为空、无 hold 计时；`dt={unmap}` 同理。
3. **TD 整条**：四字段全 `{unmap}` → 该键不出现在 `tap_dance`（断言结构，不只断言行为）。
4. **leader**：`output={unmap}` → 序列不出现在最终 `LeaderDef`。
5. **外清内补**：应用层清掉某 combo、设备层同 identity 有普通覆盖 → 最终存在（内层优先）。
6. **全局层**：`collect_unmap_warnings` 返回对应条数；且该条目按「不存在」处理、运行期不出现该字符串。
7. **`perDevice=false`**：「全局 × app」路径同样生效。
8. **回归**：现有 137 项测试全绿（`{unmap}` 不应改变任何未使用它的配置的结果）。

---

## 9. 实施顺序

1. `util.rs`（常量 + 判定）+ `runtime_builder.rs`（`build_mapping` + `apply_unmap` + 四个构建点改走它）+ 测试 1–7。
2. `config.rs` 的 `collect_unmap_warnings` + `main.rs` 打印。
3. 文档补齐：README 输出指令表加一行（`{unmap}` 与 `{fn1}`/`{Sleep N}` 并列）、CHANGELOG、`DESIGN.md` 第 8 节加一句指向本文。

> 构建提醒：`cargo build` / `test` 必须把 `CARGO_TARGET_DIR` 指到工作区外（如 `C:/Users/proje/AppData/Local/Temp/anykey-xxx`），工作区内链接可执行文件会死锁；`cargo check` 不受影响。

---

## 10. 后续可选

- 「清空此键在本应用的全部映射」批量动作：扫描基线里所有含该物理键的 combo 与各层 TD 条目，批量填 `{unmap}`，带一次确认框。纯 GUI，引擎无关。
- 若将来 TapDance 覆盖粒度改成逐字段合并（本层未写的字段继承下层），则需要同步审视本文 §4.1 与 §5.1 的单点解释假设。

---

## 11. 施工记录（2026-10-27）

落地范围与本文一致：只动引擎，配置格式、GUI、托盘都没有改。

| 文件 | 实际改动 |
|------|---------|
| `util.rs` | `UNMAP_TOKEN` + `is_unmap_output()`（trim + 大小写不敏感 + 长度严格） |
| `runtime_builder.rs` | 新增 `build_mapping()`（唯一构建入口）与 `apply_unmap()`；`build_default_device_context`、`build_multi_device_contexts`（全局预构建 + 覆盖分支）、`apply_app_override` 收尾全部改走它，顺带消掉了四处重复的「三件套」构造 |
| `state.rs` | **设计里漏掉的第 5 个构建点**：`PipelineState::new` 原来自己搓三件套，引擎启动瞬间的初始 mapping 会绕过 `{unmap}` —— 全局写错位置时该串会泄漏进管道，直到第一次应用/设备切换才被纠正。现改为走 `build_mapping` |
| `config.rs` | `unmap_warnings()`（全局层的错误位置 → 逐条文案，输出顺序稳定） |
| `main.rs` | 配置解析后 `log!("WARN {}", …)` 逐条打印 |

**测试：146 项全绿**（实施前 137 项，新增 9 项）。

- `runtime_builder_test.rs` +8：只清指定层的 combo（fn2 同键对存活）、单字段 unmap（`tap` 保留 / `hold` 置空 / `has_td()` 为假）、四字段全空整条消失、leader 序列清除、外清内补（内层加回）、全局 unmap 被丢弃并告警（含「mapping 里绝不残留该串」的不变量断言）、`perDevice=false` 路径、标记的大小写与严格性。
- `pipeline_test.rs` +1：对照组（未清除时按键被等待压住、已排计时器）vs 清除后（按下即输出自身、`timer_heap` 为空）。

**实测确认**（与 §6 的推断一致）：`normalize_key_output('{unmap}')` 原样返回；`{UNMAP}` 会被 GUI 归一化成小写 `{unmap}`；裸 `unmap` 保持裸文本（会被当文本打出 —— 可见、不静默）。

**构建**：`CARGO_TARGET_DIR` 指到 `%TEMP%`，`cargo build --all-targets` 与 `cargo test` 均无告警。
