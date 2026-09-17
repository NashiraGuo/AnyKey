# 免驱动后端（便携模式）设计

> 状态：**可实施**。定稿 2026-09-17。本文是唯一权威版；§1–§9 是设计规格，**§10 是施工日志（边做边记：进度 / 问题 / 解决）**，附录 A/B 供防止重走弯路。
> 归属：本文常驻 `main` 分支，不随代码改动回退；代码改动在 `feat/portable-backend` 分支上分段提交。

---

## 1. 一句话

AnyKey 增加第二个后端：**输入用用户态 `WH_KEYBOARD_LL` 低级键盘钩子，输出用 `SendInput`**。
免装驱动、免 testsigning、免签名、普通权限可运行、进程退出系统自动恢复。

**代价只有一件事：失去 per-device 维度。** per-app 覆盖、combo / tap-dance / leader / defer / 宏 / 文本 / 鼠标全部保留。

---

## 2. 已定决策（不要再议）

| 项 | 决定 |
|---|---|
| 吞键范围 | **吞所有键**，管道语义不变（与现有驱动后端**行为等价**，见 §3.1） |
| 输出模式 | **扫描码模式**（复用 `emit.rs` 现成函数） |
| 提权 | 不强制要求；提权窗口内映射静默不生效，这是系统行为（§3.4） |
| 驱动失败回退 | **引擎内部完成**（driver → llhook），托盘不参与 |
| 启动参数 | `--backend=driver\|llhook`，缺省 `driver` |
| 偏好存储 | `anykey_config.json` 顶层 `backend`；**GUI 写、托盘读** |
| 诊断 | **只有引擎写日志**，托盘不写 |

---

## 3. 实现依赖的机制事实

这五条决定了方案的形状，都是本项目实测或源码核实的结论。

### 3.1 输入事件可以 1:1 投影 → 下游管道零改动

驱动侧事件（`filter_driver.rs:106`）：

```rust
AnyKeyInputEvent { make_code: u16, flags: u16, device_id: u32, extra_info: u32 }
// flags: ANYKEY_KEY_BREAK=0x01, ANYKEY_KEY_E0=0x02, ANYKEY_KEY_E1=0x04  (filter_driver.rs:299-302)
```

LL hook 的 `KBDLLHOOKSTRUCT` → 同形事件：

```rust
AnyKeyInputEvent {
    make_code: hook.scanCode as u16,                                   // 裸扫描码，语义相同
    flags: if up { ANYKEY_KEY_BREAK } else { 0 }
         | if extended { ANYKEY_KEY_E0 } else { 0 },
    device_id: 0,                                                      // 便携模式 = 单设备
    extra_info: hook.dwExtraInfo as u32,
}
```

**这条也是"吞所有键"成立的原因**：驱动后端本来就是"拦截态下整设备入队并吞键"，管道再按**键名**逐个发 `Down/Up` 重放（`commit.rs:635/642` → `main.rs:480`）。所以两种后端语义一致，现有 110 项测试仍然有效。

⚠️ 已知保真度缺口：`KBDLLHOOKSTRUCT` 只有 `LLKHF_EXTENDED`（E0）位、**没有 E1 位**，所以 `ANYKEY_KEY_E1` 在便携模式下取不到值（受影响面待实测，见 §8）。

### 3.2 输出可以复用现有函数

`emit.rs:101/120` 的 `send_key_down_sendinput(scancode, extended)` / `send_key_up_sendinput()` 已经是**扫描码模式** + `KEYEVENTF_EXTENDEDKEY` + `wVk=0`，并且**返回 SendInput 计数**（0 表示被挡，调用方已有告警惯例）。
→ 键盘输出**零新增代码**；只需补鼠标的 SendInput 实现（`emit.rs` 里目前没有）。

附带好处：便携模式只剩 SendInput 一个通道，现有"SI / FLT 通道混用"的整类问题（`{Select N}` 通道边界、修饰键 up 被 IME 吞）**整体消失**。

### 3.3 自注入识别

钩子回调里 `flags & LLKHF_INJECTED` → 直接 `CallNextHookEx` 透传。
于是引擎自己的重放输出不会回灌成输入，无需 `dwExtraInfo` 标记（Kanata 同样如此，其输出侧 `dwExtraInfo` 恒为 0）。

### 3.4 未提权时，钩子收不到发往提权窗口的按键（实测）

| 观测 | 数值 |
|---|---|
| `GetLastInputInfo()` 推进次数（独立见证：确有人在输入） | 192 |
| 钩子在「前台 = 提权窗口」时的回调数 | **0** |
| 钩子在「前台 = 普通窗口」时的回调数 | 0 |
| 收尾自检（切回普通窗口注入一次，证明钩子全程存活） | 2 次回调 |

**含义**：提权窗口在前台时，钩子既看不见就不会吞 → 按键**原样生效、不丢键、不卡死**。
所以**不需要**做"前台完整性级别检测 → 自动直通"，操作系统已经给了这个语义。

### 3.5 强杀是安全的 → 换后端 = kill + start

`anykey-filter-driver/sys/anykey_flt.c:207-234` 注册了 WDF file-object cleanup 回调，源码注释原文说明它适用于 *"process exit / **forced termination**"*，实现是把所有设备的 `InterceptEnabled = FALSE`。

**含义**：进程被 `TerminateProcess` / `taskkill /f` 带走时，内核关闭句柄即触发该回调 → 拦截态立刻清零，不存在"驱动仍在拦截、键盘失灵"的窗口。
→ **不需要**优雅停止通道、不需要命名事件、不需要补发悬挂键（再按一次同键即自愈）。

---

## 4. 参数与启动流程（唯一权威版）

### 4.1 命令行

```
anykey-engine.exe <config.json> [--debug] [--backend=driver|llhook]
缺省（不传 --backend）= driver
```

| 值 | 语义 |
|---|---|
| `driver` | 优先 driver；**任一环节失败则引擎内部回退到 llhook**，不退出、不回报托盘 |
| `llhook` | **强制 llhook**，完全不尝试 driver |

两个参数都**必须扫描全部 argv**。
⚠️ 现状是位置参数解析：`args[1]` = 配置、`args[2] == "--debug"`（`main.rs:185`）——若 `--backend` 排在 `--debug` 前面，**`--debug` 会被静默丢掉**、日志不再落盘且无任何报错。必须改成 `args.iter().any(|a| a == "--debug")` 与 `strip_prefix("--backend=")`。

**为什么用 argv 而不是让引擎自己读配置里的 `backend`**：回退场景要求"生效值 ≠ 存储值"且**不能改写存储**（否则驱动恢复后回不到 driver）；argv 还让本次后端在进程命令行里可见，且零协议、无握手中间态。

**不引入 `auto`**（多一个取值没有对应需求）；也**不提供**"强制 driver、失败即退出"（日志已能说明一切，将来真有需要再加 `--backend=driver-strict`）。

### 4.2 引擎内的构造顺序（顺序本身是正确性的一部分）

```
1. 解析 argv → 缺省 driver
2. llhook 模式 → 直接装钩子（跳 4）
3. driver 模式：
   a. FilterDriver::open()
        失败 → 重试 2 次 × 500ms → 仍失败 → 记 WARN → 回退（4）
   b. set_device_intercept(all, true)
        失败 → 先 Drop 驱动后端 → 记 WARN → 回退（4）
   c. 两者都成功 → 用 driver，绝不安装钩子
4. 回退 / llhook：HookInput::install()
        成功 → 继续运行
        失败 → 记 ERROR（驱动与钩子的失败原因都写）→ 退出码 1
```

**唯一硬约束**：回退**之前**必须确认驱动后端已彻底拆掉 —— 驱动拦截与钩子吞键同时存在 = 双重吞键 = 丢键。因为回退只发生在"尚未 armed"或"armed 失败且已 Drop"之后，这一条自然满足，**但实现时不许把顺序写反**。

**为什么 open 失败要重试 2×500ms**：开机时托盘可能早于驱动服务启动；不重试会把"还没起来"误判成"不可用"，导致整场会话跑在 llhook 上（per-device 静默失效）。代价只落在失败路径。

---

## 5. 代码落点

### 5.0 后端操作面与调用路径（接缝）

#### 结论

**引擎不"查询"当前是哪个后端** —— 后端在**构造期**由 §4.2 决定，之后不可变；"当前后端"就是 `Backend` 枚举的那一个变体。运行期没有任何 `if is_llhook { … }`。

#### 操作面只有 8 个

`main.rs` 里依赖驱动的调用点全部集中在下列 8 个操作：

| 操作 | 现有调用点 | driver 实现 | llhook 实现 |
|---|---|---|---|
| 构造 | `FilterDriver::open()` `main.rs:229`（心跳线程另有 `:366`） + `register_event()` `:238` | 现状 | 装钩子 + 起消息泵线程 + 建有界通道 |
| 输入等待 | `fd.poll_all(timeout)` `:587` | `WaitForSingleObject(h_event, timeout)`，超时返回空（`filter_driver.rs:526-541`） | `rx.recv_timeout(timeout)`，`Timeout` 映射成空元组 |
| 键盘输出 | `fd.send_output(&AnyKeyOutputEvent)` `:447/450` | IOCTL | `emit.rs:101/120`（扫描码 + extended） |
| 鼠标输出 | `fd.send_mouse_output(&AnyKeyMouseOutputEvent)` `:473/537` | IOCTL | 新增 SendInput 鼠标实现 |
| 设备变更 | `fd.get_status()` `:602` 的 `ANYKEY_FLAG_DEVICE_CHANGED` 位 | IOCTL（读状态顺带清一次性标志） | **固定返回"无变更"的成功值**；⚠️ 不可返回 `Err`，否则 `:609` 每轮打日志 |
| 拦截开关 | `set_device_intercept(dev, on)` `:330/335/732/917/919` | 现状 | **空操作** —— llhook 的"闸门"就是钩子装上/卸下本身 |
| 设备枚举 | `get_device_count()` `:383` + `registry.rs:85` 的 `Registry::init(&fd)` | IOCTL | 单设备桩（`all_devices = [0]`、`default_kbd/ms = 0`） |
| 存活检查 | 心跳线程 `:366` 自己 `open` 第二个 fd | 心跳 IOCTL + 驱动 30s 看门狗 | 换成**钩子存活对账**（`GetLastInputInfo()` vs 最后一次回调时间戳） |

退出清理同样只是 `set_device_intercept(0,false)`（`:732`）→ llhook 侧在 `Drop` 里 `UnhookWindowsHookEx`。

#### 形态：enum + 薄方法（不用 trait、也不散落 if）

```rust
// src/backend.rs —— 分派只出现在这一个文件里
pub enum Backend { Driver(FilterDriver), LlHook(HookInput) }

impl Backend {
    pub fn name(&self) -> &'static str { /* "driver" / "llhook"，只给日志和状态用 */ }
    pub fn poll_all(&self, timeout_ms: u32)
        -> Result<(Vec<AnyKeyInputEvent>, Vec<AnyKeyMouseEvent>), String>   { /* match */ }
    pub fn send_output(&self, ev: &AnyKeyOutputEvent) -> Result<(), String> { /* match */ }
    pub fn send_mouse_output(&self, ev: &AnyKeyMouseOutputEvent) -> Result<(), String> { /* match */ }
    pub fn set_intercept(&self, dev: u32, on: bool) -> Result<(), String>  { /* llhook: Ok(()) */ }
    pub fn device_count(&self) -> Result<u32, String>                      { /* match */ }
    pub fn device_changed(&self) -> bool                                   { /* 取代 get_status 的那一位 */ }
    pub fn liveness_check(&self) -> LivenessStatus                         { /* 心跳 / 钩子对账 */ }
}
```

三条规则：

1. **`main.rs` 里不再出现 `filter_driver::`，也不出现任何 `match backend`。** 只出现 `backend.poll_all(...)`、`backend.send_output(...)` 这类调用。
2. **"谁控制调用路径"的答案是 `backend.rs` 一个文件。** 启动时由 `build(pref)` 选择；运行期由"构造出了哪个变体"决定。
3. **用 enum 而不是 `Box<dyn Trait>`**：只有两个实现，enum 让"只有两种后端"这个事实显式化，加第三个后端时编译器会逐个报错提醒漏处理；trait 的收益要到真有第三/第四个后端才体现，当前是过度设计。方法名尽量沿用现有语义（`poll_all` / `send_output` / `set_intercept`）以压缩 `main.rs` 的改动面。

#### `#[cfg(feature)]` 与运行期选择是两件事，不要混

- `cfg` 只回答"**这个二进制里编进了哪些后端代码**"（将来若要一个不含驱动代码的便携精简版才用得上）。
- 运行期选哪个后端由 `Backend` 枚举承担。
- 若让 cfg 去决定后端，枚举的变体就会在某些构建下消失，于是 `cfg` 会渗透进 `main.rs` 的每一处调用。
- **建议先不做 cfg 分叉**：两个后端都编进默认构建，等真有精简版需求再加。

#### 两处机械改动（容易漏）

- `drain_emit_log_flt(pipeline, fd: &FilterDriver, …)`（`:424`）与 `rematch(…, fd: &FilterDriver, …)`（`:851`）的签名都要改成 `&Backend`。
- `drain_emit_log_flt` 内部的 `TapSI / DownSI / UpSI` 分支（`:497-518`，直连 `emit.rs`）在便携模式下与主输出通道**变成同一条**，两者的差别消失 → 可在 Step 5 顺手合并简化。

#### 鼠标输出的两个实现细节

- **`button_flags` 不能直接透传给 `SendInput`**：驱动的 `MOUSE_*_BUTTON_DOWN/UP`（ntddmou `MOUSE_INPUT_DATA.ButtonFlags`）与 `MOUSEEVENTF_*` **位值不同**（整体错开一位，如 LEFT_DOWN 驱动是 `0x0001`、`SendInput` 是 `0x0002`），需要一张 12 项映射表；4/5 号键还要走 `MOUSEEVENTF_XDOWN/XUP` 并把 `mouseData` 设为 `XBUTTON1/2`。
- **绝对移动**：`SendInput` 的绝对坐标要**归一化到 0..65535**（驱动给的是像素坐标），并带 `MOUSEEVENTF_ABSOLUTE`（跨屏再加 `VIRTUALDESK`）。相对移动直接 `MOUSEEVENTF_MOVE` 即可。

### 5.1 引擎（`anykey-engine/`）

| 文件 | 改动 |
|---|---|
| **新增 `src/backend.rs`** | `enum Backend { Driver(FilterDriver), LlHook(HookInput) }` + 按 §4.2 顺序构造的入口（构造逻辑写成纯函数便于单测） |
| **新增 `src/hook_input.rs`** | 钩子安装 + **专用线程消息泵**（可复用 `app_sensor.rs:64/73-116` 的 `MsgWaitForMultipleObjectsEx` 模式）；回调内：`code != HC_ACTION` 或 `LLKHF_INJECTED` → 透传，否则按 §3.1 造事件推入**有界通道**（`try_send` 失败 → 直通降级 + 告警，**绝不在回调里等待**）；投影逻辑写成纯函数便于单测 |
| **新增 `src/sendinput_out.rs`** | 鼠标按键 / 滚轮 / 移动的 SendInput 实现；键盘直接转调 `emit.rs:101/120`；文本复用 `main.rs:736 send_unicode_text` |
| `src/main.rs` | ① 参数解析改全 argv 扫描；② 启动按 §4.2 构造后端，把现有 ~10 处 `fd.*` 调用（`:229/238/244/255/330-339/587/602/732/854/917-923`）收敛到后端句柄之后；③ 输出出口分派（driver → IOCTL / llhook → SendInput）；④ 心跳线程（`:364`）在 llhook 模式下换成"钩子存活对账"（`GetLastInputInfo()` 与会话最后回调时间戳比对，前者推进而后者不动 = 钩子已被摘 → 重装 + 记日志）；⑤ 日志改追加（§6） |
| `src/registry.rs:85` | `Registry::init(&fd)` 改为接受后端句柄；llhook 模式返回单设备桩（`all_device_ids = vec![0]`、`default_kbd/ms = 0`） |
| `src/emit.rs:68-75` | 解耦 `filter_driver::MOUSE_*`（这些是逻辑标志、两后端共用，只是放错了模块） |
| `src/state.rs:456`、`commit.rs:768/774` | `current_device` 初值 `1` 与 `emit_layer_*` 硬编码 `1` → 便携模式取 0（顺手改成取自 `key_state.device_id`，消掉硬编码） |
| `src/app_sensor.rs:28/174-184` | 目前非 filter-driver 下是 `None` 桩 → 让它在 llhook 模式也生效（**per-app 覆盖在便携模式下仍然可用**） |
| `Cargo.toml:27-29`、`lib.rs:6` | 增加 `llhook-backend` feature；两个后端可同时编译进默认构建（`filter-driver` 不再是"唯一后端"） |

### 5.2 托盘（`anykey-tray/src/engine.rs`）

**只加一项**：`start()` 里读 config 的 `backend`（**每次启动都读，不缓存**），追加 `--backend=<值>`。
不新增日志、不做回退、不做重试、不动 `ipc.rs` / `Shared`。

### 5.3 GUI（`gui/main.py`）

1. 设置界面加**后端开关**（driver / LLHook）：驱动不可用则**灰掉 + 显示具体原因**（未安装 / 已装未加载 / testsigning 未开 —— 三者处理动作不同）。
2. 设备栏"设备独立设置"开关按**当前选择的后端**联动：选 LLHook 也灰。**灰掉只禁用 UI，不清配置内容**（切回 driver 自动恢复）。
3. 保存配置时把顶层 `backend` 一并写入。
   ⚠️ 该文件的保存是"读全文件 → 改 → 整写"，它**显式保留**不认识的顶层字段（`main.py:4688-4692` 的 `debug_enabled` / `_driver_prompted`）—— 若 `backend` 不由 GUI 自己写，就必须加进那个保留块，否则拨完开关一保存就被抹掉。
4. 可选：设置页加"打开日志文件"入口（GUI 目前完全没有日志引用）。

### 5.4 配置字段

`anykey_config.json` 顶层 `backend`：`"driver"` / `"llhook"`。**字段不存在 = `driver`**（保持现状行为）。

---

## 6. 日志规格

**只有引擎写日志。** 引擎在启动横幅里打印收到的**完整 argv**，于是"请求的后端"与"生效的后端"在同一段日志里相邻。

顺带记一个既有事实：托盘的 `println!/eprintln!` 在当前架构下**全部丢弃**（`anykey-tray/src/main.rs:1` 是 `#![windows_subsystem = "windows"]`，无控制台）。所以排查只依赖引擎日志。

### 6.1 必须改：日志文件改追加

现在以 `.truncate(true)` 打开（`main.rs:193-199`），而**引擎启动恰好就是"应用后端变更"的动作** —— truncate 会把上一次运行的证据清空。改为**追加**，并在每次启动写一行分隔横幅。日志已是唯一来源，这条是硬需求。

### 6.2 记什么

| 时机 | 内容 |
|---|---|
| 启动横幅 | `===== engine start <ts> QPC=... =====` + **完整 argv**（请求值在此） |
| 参数解释 | `backend requested = driver (来源: argv / 缺省)` |
| driver 路径 | `CreateFile \\.\AnyKeyFilter` 的结果、失败时的 **Win32 错误码**（2=找不到设备、5=拒绝访问）、重试次数与结果、`set_device_intercept(true)` 返回值 |
| 回退 | `WARN fallback -> llhook (driver unavailable, last err=<code>)` —— **生效值由此行体现** |
| llhook 路径 | `SetWindowsHookExW(WH_KEYBOARD_LL)` 结果 + `GetLastError`、消息泵线程已启动、**首次收到钩子回调**（"装上"不等于"收得到"） |
| 退出 | `exit_reason` + 退出码；两处都失败时两个原因都写 |

### 6.3 怎么读

| 现象 | 结论 |
|---|---|
| 没有新的 `===== engine start =====` 行 | 引擎压根没起来（托盘没拉起 / exe 缺失），**不是后端问题** |
| 有 `requested = X`、**没有** `fallback` 行 | 请求的后端**直接生效** |
| 有 `WARN fallback -> llhook` | 驱动本次没用上，原因看紧跟的错误码；per-device 不生效属**预期** |

---

## 7. 实施顺序

| 步骤 | 内容 |
|---|---|
| **Step 0** | `main.rs` 参数解析改全 argv 扫描（修 `--debug` 被静默丢掉的隐患）+ 日志改追加。**先做的理由**：修的是现存隐患，不依赖任何新功能，风险最低 |
| Step 1 | `backend.rs` + `hook_input.rs`：钩子 + 消息泵 + 空白透传通路 + 1:1 投影，用最小映射（单键）跑通端到端 |
| Step 2 | `sendinput_out.rs`（鼠标）+ 文本 / `{Select N}` 验证 |
| Step 3 | 日志补齐（requested / fallback / 首次回调 / exit_reason） |
| Step 4 | 托盘传参 + GUI 开关 + 配置字段 |
| Step 5 | 收尾解耦（`registry` / `app_sensor` / `emit` / `current_device`） |
| Step 6 | 打包便携 ZIP（不含 `anykeyFilterDriver/` 与 `安装驱动.bat`）+ README 能力对照表 |

---

## 8. 必须实测（结果可能回改设计）

| 项 | 为什么关键 |
|---|---|
| **Alt+Tab / Win+D / Win+E / Win+方向键贴靠** | 物理 Alt 与 Tab 被拆成两次投递后，系统还认不认这是一个组合 —— 便携模式最脆的地方 |
| **AltGr / 右 Alt**（E0 0x38） | Kanata 官方已知问题里就列了 AltGr 异常 |
| **CapsLock / NumLock / ScrollLock**：重放是否恰好翻转一次、LED 与状态是否一致 | 重放模型下最容易"翻两次"或"不翻" |
| **非美式布局下的原样透传是否字符级一致**（Dvorak / AZERTY） | 物理 scan → 解析键名 → 反解 scan → 重放 → 出字符，首尾必须相同；吞所有键会把这个表的任何缺陷放大到每一个按键 |
| **中文 IME**：组合串、候选框、密码框、网银控件 | 所有键都变成注入键 |
| **延迟**（每键多一跳） | 用 `tools/keylatency` 探针测 |
| **Pause/Break（E1 键）保真度** | §3.1 的已知缺口，确认影响面是否只限这一个键 |
| **快速用户切换 / 锁屏 / 睡眠唤醒** | 钩子是 per-desktop/session；确认恢复后状态一致、无卡键 |
| **自动化测试通路** | 用**驱动在 dev 模式注入"无注入标记"的键**来驱动便携后端的钩子（驱动注入等同物理键）；投影逻辑另写纯函数做单测，否则每次回归都要人工敲键盘 |

---

## 9. 已知限制（写进 README）

- **便携模式 = 全键盘注入**：反作弊、过滤注入输入的软件、部分密码/安全控件可能拒绝输入。这是它相对驱动后端最本质的差异（驱动注入不带注入标记）。
- **与其它 LL hook 工具不共存**（AutoHotkey / espanso / 输入法的 hook 功能）：安装顺序决定效果，可能出现热键失灵或重复触发。
- **Win+L、Ctrl+Alt+Del、安全桌面（UAC 提示、登录界面）永远拦不到。**
- **多键盘共用同一套映射**（无 device 维度）；GUI 在便携模式下应隐藏/禁用 device 相关设置。
- **不提权运行时，提权窗口内映射静默不生效**（按键仍原生工作）；需要生效就以管理员身份启动 AnyKey。
- **鼠标建议不接管移动**（高频事件走"吞 + 重放"是性能灾难；Kanata 为此专门做了位移累加）。

---

## 10. 实施记录（进度 / 问题 / 解决）

> 边做边追加，只增不改。每条记：**做了什么 / 遇到什么 / 怎么解决 / 下一步**。

### 2026-09-17 基线

**做了**：确立工作流并落地第一层保险。

- 仓库状态：分支 `main`，HEAD `ef7f36c3`（与 `origin/main` 同步）；工作区无修改，仅 3 个未跟踪项（本设计文档、`tools/`、`build/release.zip`）。
- 动手前快照：`backups/20260917_portable_backend/`（649.4 KB，含 `MANIFEST.txt` 记录 HEAD）—— 覆盖 git 管不到的部分。
- 工作流：**文档留 `main`（本文件是活日志，不随代码回退）**；代码在 `feat/portable-backend` 分支上按"可编译 + 可独立解释"的边界分段提交；每段结束必须编译 + 跑测试。
  - ⚠️ **已变更**：当天晚些时候因 `git switch` 事故（见下方事故条目）改为**不切分支**的提交方式 —— 文档用 `AnyKey/tools/doc_to_main.py` 直接提交到 `main`。

**遇到的问题 / 决策**：

- 用户指出文档"需要在工作时记录每一步进展、问题、解决方法，不适合回退" → 因此文档归属 `main`，且新增本节作为施工日志。
- `build/release.zip` 未被忽略（`.gitignore` 只忽略了 `build/release/` 目录和 `AnyKey_v*.zip`）→ 补 `build/*.zip`，避免它污染 `git status`、防止误提交。

**下一步**：Step 0（`main.rs` 参数解析改全 argv 扫描 + 日志改追加模式）。

### Step 0 —— 参数解析 + 日志追加（提交 `03d4e2a`，分支 `feat/portable-backend`）

**做了什么**

- `main.rs:185` 参数解析：`args.get(2).map_or(false, |a| a == "--debug")` → `args.iter().any(|a| a == "--debug")`，去掉位置依赖（这个隐患是：将来插入 `--backend=...` 会让 `--debug` 被静默丢掉、日志不再落盘且无报错）。
- `main.rs:195` 日志打开：`create(true).write(true).truncate(true)` → `create(true).append(true)`。
- 启动时写分隔横幅：`===== engine start <ts> =====` + `argv = [...]`（`ts_tag()` 提供墙钟 + QPC，与 `tools/keylatency` 同一时间基）。
- `.gitignore` 补 `build/*.zip`（只在忽略规则里，见基线条目）。

**验证**：`cargo build` 通过（17.67s）；`cargo test` **110 项全绿**（原文档写"109 项"，实测为 110，已就地修正）。

**遇到的问题**

- `cargo test` 期间刷出一批 `failed to garbage collect finalized incremental compilation session directory … 拒绝访问 (os error 5)`。是增量编译目录被占用（有进程锁着 `target/` 或杀软扫描），**不影响编译与测试结果**，本次不处理。
- 本轮 shell 环境里 `ls/grep/tail/sed/cat` 等 coreutils 时有时无（`PATH` 被包装脚本影响），已改用 Python 做文件操作、`git` 直连——**后续脚本一律走 Python，不依赖 coreutils**。

**为什么先做这一段**：它修的是现存隐患、不依赖任何新功能、行为影响面最小（只影响参数解析与日志），因此最适合用来验证整条"分支 + 分段提交 + 每段编译测试"的工作流本身是否顺。

**下一步**：Step 1 —— `backend.rs`（`enum Backend` + 8 个薄方法，driver 变体先只包住现状；**要求行为完全不变**）+ 参数加 `--backend=driver|llhook`。

### Step 1 —— 引入 `Backend` 抽象（提交 `b85762c`，**等价重构**）

**做了什么**

- 新增 `anykey-engine/src/backend.rs`：`enum Backend { Driver(FilterDriver) }` + 8 个薄方法
  （`name` / `poll_all` / `send_output` / `send_mouse_output` / `set_intercept` / `device_count` / `enum_devices` / `device_changed`），
  **`match` 只出现在这一个文件里**。故意没有预留任何无人调用的方法。
- `lib.rs`：按 `filter_driver` 同样的 `#[cfg(feature = "filter-driver")]` 声明 `pub mod backend`。
- `main.rs`：构造期（`open` / `register_event` / 首次排空）仍用具体 `FilterDriver`，之后 `let backend = Backend::Driver(fd);`
  并打一行 `Backend: driver`；此后约 15 处调用点全部改为 `backend.*`；`drain_emit_log_flt` 与 `rematch` 的签名
  由 `&FilterDriver` 改为 `&Backend`；热插拔那种"读状态取一次性标志"的逻辑收进 `Backend::device_changed()`，
  `ANYKEY_FLAG_DEVICE_CHANGED` 不再出现在 `main.rs`。
- `registry.rs`：`scan_all` / `init` / `refresh` 改收 `&Backend`。

**验证**

- `cargo build` 通过（35.57s，**零警告**）；`cargo test` **110 项全绿**（16 组）。
- 行尾自检：本次用 Python 整文件重写，特意核对了 CRLF 计数（全部为 0，纯 LF），`git diff --stat` 仅 +135/−35 ——
  没有出现"整文件行尾变化淹没 diff"这种意外。

**遇到的问题**

- **管道不可靠**：`cargo test ... | python 解析` 拿到 0 行（本机 shell 包装脚本会干扰管道），已改为**落盘再解析**。
- 解析 `test result:` 行时第一版用 `split()[-1]`，取到的是 `passed` 而不是数字 → 改用正则。教训：**统计脚本自己也要先验证一次**。

**待补的验证（无法自动）**：驱动模式冒烟 —— 构建 release 后由托盘重载，确认日常使用一切照旧（本段是等价重构，风险低但应确认）。

**下一步**：Step 2 —— `hook_input.rs`（钩子 + 专用线程消息泵 + 有界通道）与 `sendinput_out.rs`（SendInput 鼠标），并在 `backend.rs` 加 `LlHook` 变体。

### 2026-09-17 事故：`git switch` 让整个 `anykey-engine/src/` 从磁盘消失（两次）

**现象**

- 第一次：一条链式命令（`git switch -q main` + 文档补丁 + `git commit` + `git switch` 切回）执行中被 SIGTERM 打断；
  `anykey-engine/src/` **整棵目录从磁盘消失**，HEAD 仍停在原分支，并留下 **0 字节的 `.git/index.lock`**。
- 第二次：在**工作区干净、无锁、内容已验证一致**的情况下，单独一条 `git switch -q main` 同样被 SIGTERM，`src/` 再次整棵消失。

**根因（实测归纳）**

本环境中 **git 驱动的文件删除**会触发进程被杀。两次失败都发生在 `git switch` 需要**删除**分支独有文件（`backend.rs`，main 上没有）的那一刻；
而纯创建/覆盖写从未失败过（`git restore --source=HEAD --worktree -- <path>`、`git add`、`git commit`、`git read-tree`、`git update-index` 全部正常）。
这与本项目历史上 **`git rm` 导致整个目录树消失**是同一类问题。

**恢复配方（第二次已验证）**

1. 确认没有 git 进程 → 删除陈旧 `.git/index.lock`（本次为 0 字节，确认无进程后可安全删）；
2. `git read-tree HEAD` —— 只重置**索引**（清掉被中断操作留下的暂存删除），不动工作区；
3. `git restore --source=HEAD --worktree -- anykey-engine/src` —— **只创建**，不删除；
4. **行尾归一化**：把工作区文件写成与库内 blob **逐字节一致**。因为 `core.autocrlf=true` 会让 `git restore` 写出 CRLF，
   而 `git switch` 的安全检查**不认 CRLF 为"未修改"**（`git diff` 却会归一化后认为无差异）—— 两者判定不一致会产生
   "幽灵修改"，既阻止切换也让人误以为有改动；
5. `git update-index --refresh`，然后 `git status` 应只剩有意未跟踪的 `tools/`。

**流程变更：不再使用 `git switch`**

文档改由 `AnyKey/tools/doc_to_main.py` 提交到 `main`，原理是 git 底层命令：

```
hash-object -w                      # 工作区文档 → blob
read-tree main                      # 用**临时索引**(GIT_INDEX_FILE 隔离)取出 main 的树
update-index --cacheinfo 100644,<blob>,<path>
write-tree                          # 新树
commit-tree <tree> -p main -m "…"   # 以 main 为父
update-ref refs/heads/main <commit> # 移动 main 指针
```

全程只有"写对象 / 改索引 / 移指针"，**不碰工作区、不删除任何文件**。文档在 `main` 与分支上内容保持一致
（两边 blob 相同，故日后合并该文件不冲突）；代码仍只在分支上提交。

**教训**

1. **一次只跑一条 git 命令，不链式**（链式一旦中断，症状是"目录消失 + 陈旧锁"，排查成本高）。
2. **任何"会删除工作区文件"的 git 操作在这台机器上都按危险操作对待** —— `git switch` 也要算进去，不只是 `git rm`。
3. 用脚本改写文件时**保留或归一化行尾**；`git diff` 与 `git status/switch` 对 CRLF 的判定不一致，不能只看其中一个。

## 附录 A：被否决的路线（一句话，防止重走）

| 路线 | 否决原因 |
|---|---|
| `RIDEV_NOLEGACY` 当闸门 | 它砍掉 legacy 生成，而 `SendInput` 重放也必须走这一步 → **自己关掉自己的重放**（实测：NOLEGACY 下物理键与注入键 `WM_KEYDOWN` 均为 0） |
| hook 吞键 + Raw Input 拿设备身份 | 吞键是**终止**操作：被吞的键**不再产生 `WM_INPUT`**（双进程实验 + AutoHotkey 独立实现交叉验证） |
| 用户态 HID 直读键盘 | 系统对系统键盘/鼠标独占读写，`CreateFile` 读权限一律 Access Denied（管理员 / SYSTEM 同） |
| ④层 DLL 注入（HIDeous 式） | 能用且能拿设备身份，但全局注入带来 AV 误报、32/64 双份 DLL、UIPI —— 对开源分发不可接受 |
| `RegisterHotKey` 当免注入闸门 | 只有 down 语义、无 keyup → 修饰键 / 长按 / tap-dance / combo 全做不了 |
| 做成 TSF 输入法 | 载荷无设备字段（SDK 头文件原文）；按键接入槽是**单占位**、无"向下转交"原语；且必须数字签名 |
| 键盘布局（HKL） | 只能改"这个键是什么意思"，**不能改"这个键存不存在"**（`_none_` 实测仍发 `WM_KEYDOWN vk=0xFF`）；且静态、per-thread、需管理员安装 |
| 鼠标侧 | **不对称已实测**：钩子吞掉鼠标移动后 RawInput **仍照常投递**（光标冻结 Δ=0 而 RawInput 收到 327 包）→ 免驱动 + 鼠标按设备识别**机制上成立**，本轮不做 |

---

## 附录 B：探针索引（`AnyKey/tools/`，均保持未跟踪）

| 探针 | 证明了什么 |
|---|---|
| `probe_elev_hook2.py` | 未提权钩子收不到发往提权窗口的按键（`GetLastInputInfo` 独立见证 + 收尾自检排除钩子中途失效） |
| `probe_dual.py` / `probe_ahk_cross.py` | 钩子吞键 → 被吞的键不再产生 `WM_INPUT`（双进程对照 + AHK 交叉验证） |
| `probe_hook_combo.py` / `probe_hook_pump.py` | 同进程内注册键盘 Raw Input 会切断 LL hook 投递（与线程/顺序/窗口无关，可逆） |
| `probe_mouse_swallow.py` | 鼠标移动与键盘**不对称**（吞掉后 RawInput 仍投递） |
| `probe_layout_erase.py` / `probe_layout_raw.py` | 布局把 `_none_` 变成 `vk=0xFF` 而非丢弃；布局改写发生在 RawInput 分支之前 |
| `probe_kbdvsc.py` | 全量读 218 个布局的 `pusVSCtoVK`（判断"布局能否表达 X"） |
| `probe_nolegacy.py` | `RIDEV_NOLEGACY` 会连带砍掉 `SendInput` 重放 |
| `probe_tsf_profiles.py` | TSF 同一时刻只有一个 active profile |
