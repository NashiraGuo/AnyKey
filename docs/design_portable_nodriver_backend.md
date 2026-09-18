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
| 鼠标范围 | **按键 / 滚轮拦截重放；移动一律放行**（与驱动一致：纯移动直接转发、不入管道）。不注册 Raw Input，也不做鼠标按设备识别 |
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

### 3.6 键盘钩子与鼠标钩子可以在同一进程/同一线程共存（实测）

便携模式要在已有的键盘钩子基础上再加一个鼠标钩子，所以这条必须先钉死 —— 否则做出来会是
"装了鼠标就不能用键盘"。

`tools/probe_llmouse.py`（两个钩子装在同一线程、同一个消息泵）实测五项全过：

| 判定 | 结果 |
|---|---|
| 装上鼠标钩子后，键盘钩子仍收到回调 | ✅ |
| 鼠标钩子能看到按键事件（注入点击 → `ms_hook_edge_injected=2`） | ✅ |
| 鼠标钩子能看到移动事件 | ✅ |
| 放行的注入移动真的移动了光标（`+40` → 光标 `+30`） | ✅ |
| 放行的注入点击到达了窗口（窗口收到 `WM_LBUTTONDOWN`） | ✅ |

顺带两个实测细节：① 相对移动会被系统的"指针加速"缩放，注入 `dx=40` 光标只走 30 —— 所以
判定移动是否放行要看**方向和量级**，不能要求精确等于注入值；② 这条与 §3 里"注册**键盘**
Raw Input 会切断 LL hook"并不矛盾：那是 Raw Input 注册的副作用，不是钩子之间的干扰。

便携模式因此**完全不需要 Raw Input**（它没有设备维度，设备身份没有用处），
顺带避开了上面那个坑。

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
| 构造 | `FilterDriver::open()` `main.rs:229`（心跳线程另有 `:366`） + `register_event()` `:238` | 现状 | 装**两个**钩子（键盘 + 鼠标）+ 起消息泵线程 + 建有界通道 |
| 输入等待 | `fd.poll_all(timeout)` `:587` | `WaitForSingleObject(h_event, timeout)`，超时返回空（`filter_driver.rs:526-541`） | `rx.recv_timeout(timeout)`，`Timeout` 映射成空元组 |
| 键盘输出 | `fd.send_output(&AnyKeyOutputEvent)` `:447/450` | IOCTL | `emit.rs:101/120`（扫描码 + extended） |
| 鼠标输入 | 与键盘同一次 `poll_all`（驱动是两个队列、同一次 poll 取走） | IOCTL | `WH_MOUSE_LL`，**纯移动放行、按键/滚轮入队**（与驱动同语义） |
| 鼠标输出 | `fd.send_mouse_output(&AnyKeyMouseOutputEvent)` `:473/537` | IOCTL | `sendinput_out.rs`（SendInput） |
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

#### 输出接缝：两处机械改动 + 一处刻意不合流

- ✅ `drain_emit_log_flt` 与 `rematch` 的签名已改成 `&Backend`（Step 1）。
- ⏸ `drain_emit_log_flt` 内部的 `TapSI / DownSI / UpSI` 分支与主输出通道**没有合并** —— 这是决定，不是遗漏（Step 5 复核后维持）：
  便携模式下两者确实都落到 SendInput，但 ① 日志标签（`send(SI)` vs `send(FLT)`）是**诊断配方**的一部分（§6.3），合并等于换掉读法；
  ② `Fence` 的时长是 commit 阶段按 FLT 积压量算出来的**决策**，要动就得动管道。收益只是少一段重复代码，不值得碰输出路径。

#### 鼠标输出的两个实现细节

- **`button_flags` 不能直接透传给 `SendInput`**：驱动的 `MOUSE_*_BUTTON_DOWN/UP`（ntddmou `MOUSE_INPUT_DATA.ButtonFlags`）与 `MOUSEEVENTF_*` **位值不同**（整体错开一位，如 LEFT_DOWN 驱动是 `0x0001`、`SendInput` 是 `0x0002`），需要一张 12 项映射表；4/5 号键还要走 `MOUSEEVENTF_XDOWN/XUP` 并把 `mouseData` 设为 `XBUTTON1/2`。
- **绝对移动**：`SendInput` 的绝对坐标要**归一化到 0..65535**（驱动给的是像素坐标），并带 `MOUSEEVENTF_ABSOLUTE`（跨屏再加 `VIRTUALDESK`）。相对移动直接 `MOUSEEVENTF_MOVE` 即可。

### 5.1 引擎（`anykey-engine/`）

| 文件 | 改动 |
|---|---|
| **新增 `src/backend.rs`** | `enum Backend { Driver(FilterDriver), LlHook(HookInput) }` + 按 §4.2 顺序构造的入口（构造逻辑写成纯函数便于单测） |
| **新增 `src/hook_input.rs`** | **`WH_KEYBOARD_LL` + `WH_MOUSE_LL` 两个钩子同线程安装**（§3.6 实测可共存）+ **专用线程消息泵**（`MsgWaitForMultipleObjectsEx` 模式，同 `app_sensor.rs:64/73-116`）；回调内：`code < 0` 或 `LLKHF_INJECTED`/`LLMHF_INJECTED` → 透传，否则按 §3.1 造事件推入**有界通道**（`try_send` 失败 → 直通降级 + 告警，**绝不在回调里等待**）；鼠标**移动**不进管道（与驱动「纯移动直接转发」一致）；投影逻辑写成纯函数便于单测 |
| ✅ `src/hook_input.rs`（Step 6 补充） | **三个开关 + 一个纯判定**：`capture`（镜像入队）/ `swallow`（吞原事件）/ `accept_injected`（收下注入事件），由 `decide() -> {Pass, Mirror, Swallow}` 合成唯一判定；`HookOptions::production()` 与 `::for_test()` 是仅有的两种合法组合。原先「入队」与「吞键」焊在一起（A2），于是**无法只观察不吞** —— 而这正是自动化测试需要的能力。驱动后端一直有这个二分（`InterceptEnabled` / `CaptureEnabled`），所以这里是让两个后端对称，而不是为测试开特例。另加**紧急退出组合键**（LCtrl+Space+Esc，回调内先于一切判定的纯函数位运算）—— 动机、语义与实现见 §10 |
| **新增 `src/sendinput_out.rs`** | 鼠标按键 / 滚轮 / 移动的 SendInput 实现；键盘直接转调 `emit.rs:101/120`；文本复用 `main.rs:736 send_unicode_text` |
| `src/main.rs` | ① 参数解析改全 argv 扫描；② 启动按 §4.2 构造后端，把现有 ~10 处 `fd.*` 调用（`:229/238/244/255/330-339/587/602/732/854/917-923`）收敛到后端句柄之后；③ 输出出口分派（driver → IOCTL / llhook → SendInput）；④ 心跳线程（`:364`）在 llhook 模式下换成"钩子存活对账"（`GetLastInputInfo()` 与会话最后回调时间戳比对，前者推进而后者不动 = 钩子已被摘 → 记日志告警）；⑤ 日志改追加（§6） |
| ✅ **新增 `src/events.rs`** | 两个后端共用的 I/O 词汇，**无 cfg** —— 详见 §5.4（Step 5） |
| ✅ `src/registry.rs` | `scan_all` / `init` / `refresh` 已收 `&Backend`（Step 1）；本步把设备清单负载（`AnyKeyDeviceInfo` / `AnyKeyEnumDevicesRequest`）的 import 改指 `events`。llhook 侧返回单设备桩（device 0，同时标记键盘+鼠标） |
| ✅ `src/emit.rs` | `MOUSE_*` 的 import 改指 `events`（Step 5） |
| ✅ `src/state.rs`、`commit.rs` | `current_device` 初值 `1` → `0`；`emit_layer_act/deact` 的硬编码 `1` → `self.current_device`（Step 5，两处都只是「值不说谎」，行为无变化 —— 实测驱动会话 46900 行里真实输出恒为 `dev=4/5`，占位值从不出现） |
| ✅ `src/app_sensor.rs` | 摘掉全部 4 处 `cfg(feature = "filter-driver")` 与 `#[cfg(not(…))]` 的 `None` 桩（Step 5）—— 它只用 `SetWinEventHook` + `GetForegroundWindow`，**本来就与后端无关**；per-app 覆盖在两个后端下走同一份代码 |
| ⏸ `Cargo.toml`、`lib.rs` | **不加 `llhook-backend` feature、不做 cfg 分叉** —— 理由见 §5.4 末段 |

> ⚠️ 上表 ④ 的**「重装钩子」部分未实现**：存活对账只写日志告警，没有重装逻辑。见 §10 Step 6 的遗留项。

### 5.2 托盘（✅ 已实施）

**只加一项**：`start()` 里读 config 的 `backend`（**每次启动都读，不缓存**），追加 `--backend=<值>`。
不新增日志、不做回退、不做重试、不动 `ipc.rs` / `Shared`。

落实为两个文件：新增 `src/config.rs`（只读那几个「机器属性」字段：`load_bool` / `load_backend` + 纯函数 `normalize_backend`，带 3 个单测），`engine.rs` 里 3 行调用。
**新开模块的原因**：`engine.rs` 读一个字段不该反向依赖 `app.rs`；顺带把 `app.rs::load_debug_flag` 也改为复用 `config::load_bool`，去掉第二处 `serde_json` 取值。

### 5.3 GUI（✅ 已实施）
四条全部落地，位置在侧栏「**运行模式**」与「设备设置」两个小节（后端开关放在设备开关之上 —— 它决定设备维度能不能用，联动关系一眼可见）：
1. 设置界面加**后端开关**（driver / LLHook）：驱动不可用则**灰掉 + 显示具体原因**（未安装 / 已装未加载 / testsigning 未开 —— 三者处理动作不同）。
2. 设备栏"设备独立设置"开关按**当前选择的后端**联动：选 LLHook 也灰。**灰掉只禁用 UI，不清配置内容**（切回 driver 自动恢复）。
3. 保存配置时把顶层 `backend` 一并写入。
   ⚠️ 该文件的保存是"读全文件 → 改 → 整写"，它**显式保留**不认识的顶层字段（`main.py:4688-4692` 的 `debug_enabled` / `_driver_prompted`）—— 若 `backend` 不由 GUI 自己写，就必须加进那个保留块，否则拨完开关一保存就被抹掉。
4. ✅ 落地为准：开关 = 「便携模式」ON/OFF（ON = `llhook`）；说明行**始终有内容**（驱动模式 / 便携模式 / 驱动不可用+具体动作），让「这个开关是什么」自解释；**每次点「刷新」都重探一次驱动**（用户刚装完驱动 → 不必重启 GUI）。「打开日志文件」入口未做，留待后续。
5. **判据唯一**：新增 `_device_ui_enabled()` = `驱动可用 and 非便携 and perDevice`，所有置灰/恢复都从它派生（本项目着色类 bug 的典型来源是「同一属性两个判据」）。
6. 附带清理：`_toggle_per_device` 拆成 `_restyle_device_rows()` + 一行落盘，避免「重画」与「写盘」耦在一个函数里（原来的 `self._schedule_autosave()` 在函数末尾，任何重画都会顺带触发一次保存）。

`anykey_config.json` 顶层 `backend`：`"driver"` / `"llhook"`。**字段不存在 = `driver`**（保持现状行为）；**取值非法也退回 `driver`** —— 一个字段的笔误不该让引擎起不来。✅ 已实施。写入规则：驱动可用时由开关决定；**驱动不可用时开关被禁用 → 保留原存储值**（引擎自己会回退到便携模式，所以「运行=便携、存储=driver」自洽，驱动恢复后自动回 driver）。

### 5.4 模块边界（Step 5 定稿）

**判据（一句话）**：某个类型/常量**在免驱动后端下有对应物吗**？有 → `events.rs`；没有 → `filter_driver.rs`。

| 模块 | 内容 | cfg |
|---|---|---|
| ✅ **`src/events.rs`**（新增，216 行） | 四个事件结构体（`AnyKeyInputEvent` / `AnyKeyOutputEvent` / `AnyKeyMouseEvent` / `AnyKeyMouseOutputEvent`）、设备清单负载（`AnyKeyDeviceInfo` / `AnyKeyEnumDevicesRequest`）、`ANYKEY_KEY_*`、`MOUSE_*`（按钮 + 移动标志）、`ANYKEY_DEV_FLAG_*`、`MouseEventTranslator` | **无** |
| ✅ `src/filter_driver.rs`（763 → 546 行） | 驱动协议：IOCTL 码、`FilterDriver` 句柄与全部方法、状态/拦截请求结构、`ANYKEY_FLAG_DEVICE_CHANGED`（2026-09-18 v0.5 又删掉心跳与 `ANYKEY_STATE_*`，见 §10） | `filter-driver` |
| ✅ `src/hook_input.rs`、`src/sendinput_out.rs` | 免驱动输入 / 输出 | **无**（本步摘掉） |
| ✅ `src/app_sensor.rs` | 前台窗口感知 | **无**（本步摘掉） |
| `src/backend.rs` | 后端抽象 | `filter-driver` —— 只有 `Driver` 变体需要驱动句柄；同一个 enum 不能一半带特性一半不带 |

**为什么 `MouseEventTranslator` 也算共享词汇**：它把合并鼠标包展开成单边沿事件，纯 `HashMap` 逻辑、不碰驱动句柄；llhook 的鼠标事件同样要过它（统一入口比按后端分叉更不容易漏）。

**为什么不引入 `llhook-backend` feature / 不做 cfg 分叉**（刻意）：

- §5.0 已定：**不让 cfg 决定运行期用哪个后端**；cfg 只该回答"这个二进制里编进了哪些后端代码"。
- 而现在**没有这样的构建目标**：`main.rs` 的 `fn main` 整体挂在 `filter-driver` 下，`registry` 的设备清单也来自驱动。真要出无驱动构建得先解耦这两处，而发布包只有一个二进制，收益为零。
- 本步的实际目标是"**cfg 不再说谎**"，已达成：`hook_input` / `sendinput_out` / `app_sensor` 与特性无关；**全仓 `filter_driver::` 引用只剩 `FilterDriver`（10 处）与 `ANYKEY_FLAG_DEVICE_CHANGED`（1 处）** —— 也就是真正只属于驱动的东西。

---

## 6. 日志规格

**只有引擎写日志。** 引擎在启动横幅里打印收到的**完整 argv**，于是"请求的后端"与"生效的后端"在同一段日志里相邻。

顺带记一个既有事实：托盘的 `println!/eprintln!` 在当前架构下**全部丢弃**（`anykey-tray/src/main.rs:1` 是 `#![windows_subsystem = "windows"]`，无控制台）。所以排查只依赖引擎日志。

### 6.1 日志文件改追加（✅ 已实施）

已从 `.truncate(true)` 改为 **append**，并在每次启动/收尾各写一条横幅：
`===== engine start <ts> =====` / `===== engine stop <ts> (backend=…) =====`。
理由：引擎启动恰好就是"应用后端变更"的动作，truncate 会把上一次运行的证据清空。
日志是**唯一**诊断来源（托盘不写日志），所以这条是硬需求。

### 6.2 记什么

| 时机 | 内容 |
|---|---|
| 启动横幅 | `===== engine start <ts> QPC=... =====` + **完整 argv**（请求值在此） |
| 参数解释 | `backend requested = driver (来源: argv / 缺省)` |
| driver 路径 | `CreateFile \\.\AnyKeyFilter` 的结果、失败时的 **Win32 错误码**（2=找不到设备、5=拒绝访问）、重试次数与结果、`set_device_intercept(true)` 返回值 |
| 回退 | `WARN fallback -> llhook (driver unavailable, last err=<code>)` —— **生效值由此行体现** |
| llhook 路径 | `SetWindowsHookExW(WH_KEYBOARD_LL + WH_MOUSE_LL) -> OK`；失败则带 `GetLastError`；`message pump thread started`；**`first hook callback received (kb_callbacks=N mouse_edges=M, Xms after install)`**（"装上"不等于"收得到"）；后续心跳 `llhook: liveness …`，异常时 `WARN … 钩子已 Nms 无回调`、恢复时 `hook callbacks resumed` |
| 退出 | `exit_reason = <原因>`（非零退出码时附 `exit code = N`）+ 收尾横幅。可取的值：`config_read_failed` / `config_invalid` / `no_backend_available (exit code = 1)` / `poll_all_failed: …`。两处都失败时两个原因各写一行 |

### 6.3 怎么读

| 现象 | 结论 |
|---|---|
| 没有新的 `===== engine start =====` 行 | 引擎压根没起来（托盘没拉起 / exe 缺失），**不是后端问题** |
| 有 `requested = X`、**没有** `fallback` 行 | 请求的后端**直接生效** |
| 有 `WARN fallback -> llhook` | 驱动本次没用上，原因看紧跟的错误码；per-device 不生效属**预期** |
| 有 `first hook callback received` | 钩子链路确实通。括号里哪个计数先非零 = 键盘还是鼠标先到。⚠️ **`mouse_edges` 也计注入事件**，所以这一行只证明"钩子被调用过"；要判断"物理键能到"得看 `kb_callbacks`（这正是 §3.4 提权窗口问题的判据） |
| 日志尾部**既无** `exit_reason` **也无** `engine stop` | 进程被**外部强杀**（托盘的重载/暂停/退出都走 `taskkill`）—— 这是正常路径，不是故障 |
| 尾部有 `exit_reason = config_read_failed` / `config_invalid` | 配置文件的问题，与后端无关 |
| 尾部有 `exit_reason = no_backend_available (exit code = 1)` | 驱动与钩子都起不来；具体两条原因在它上面几行 |

> 注：配置类失败当前以 `return` 结束，**进程退出码为 0**。若将来托盘想靠退出码区分"配置错"与"正常结束"，需改成非零 —— 留到 Step 4 动托盘时一起定。

---

## 7. 实施顺序

| 步骤 | 内容 |
|---|---|
| **Step 0** | `main.rs` 参数解析改全 argv 扫描（修 `--debug` 被静默丢掉的隐患）+ 日志改追加。**先做的理由**：修的是现存隐患，不依赖任何新功能，风险最低。✅ 已做，提交 `03d4e2a` |
| Step 1 | `backend.rs`：`enum Backend` + 薄方法，**行为完全不变**（等价重构）。✅ 已做，提交 `b85762c` |
| Step 2 | `hook_input.rs`（键盘+鼠标两个钩子 + 消息泵 + 有界通道 + 1:1 投影）+ `sendinput_out.rs`（鼠标输出）+ `Backend::LlHook` 变体 + `--backend=` 参数与引擎内回退。✅ 已做，提交 `78a5e39` / `b16c231` |
| **Step 2c** | **实机冒烟测试**（`tools/smoke_llhook.py`）：键盘重放 / hold 层 / 鼠标拦截重放 / 键盘→鼠标输出，四项全通。✅ 已做（见 §10） |
| Step 3 | 日志补齐（requested / fallback / **首次钩子回调** / exit_reason + 收尾横幅）。✅ 已做，提交见 §10；核对脚本 `tools/check_logging_step3.py`（全自动） |
| Step 4 | 托盘传参（`config.rs` + `engine.rs`）+ GUI 开关 + 配置字段。✅ 已做，提交见 §10；验证脚本 3 个（见附录 B） |
| Step 5 | 收尾解耦（`registry` / `app_sensor` / `emit` / `current_device`；四个事件结构体搬进 `src/events.rs`，两个新模块不再与 `filter-driver` 同 cfg）。✅ 已做，提交 `18447ad`，详见 §10 与 §5.4 |
| Step 6 | ~~打包便携 ZIP（不含 `anykeyFilterDriver/` 与 `安装驱动.bat`）~~ + README 能力对照表。**打包策略已改为「不分包」（用户决定，2026-09-18）**：发布包只出一种、全带三项 —— 没装驱动的用户直接用 `anykey/` 里的 GUI 即可（引擎自动回退便携模式），因此**不需要便携专用包**，`build_release_package.py` 无需改动。✅ README 对照表已补（中英双语，见 §10） |
| **人工验证** | 每个涉及输入的步骤之后都要做一次**实机**冒烟（敲键盘确认映射生效）—— 自动化测试覆盖不到钩子本身。✅ 已完成一次（Step 2c）；**需要人配合时，启动前必须先用提问模式确认用户就在电脑前** |

> **构建注意（本机环境，2026-09-17 实测）**：`cargo build` / `cargo test` 要把 target 移出工作区 ——
> `CARGO_TARGET_DIR=C:/Users/proje/AppData/Local/Temp/<name> cargo test`。否则**在工作区内链接可执行文件会死锁**
> （产物已写出、rustc 永不退出）。机制与诊断见 §10 的 Step 2 记录。

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
| **自动化测试通路** | ✅ **已做，且不必用驱动**（Step 6）：`tests/llhook_backend_test.rs` 用 `HookOptions::for_test()`（只观察不吞 + 接受注入）装钩子，再用 `SendInput` 注入 F13/滚轮，验证 投影 → 通道 → `poll_all` → 管道映射 → 输出回流。**全自动、不需要驱动/管理员/人工**，且不影响本机键盘。原计划的「用驱动注入无标记键」仍可作为另一种路径（`tools/probe_drv_inject_to_hook.py`，需要退出 AnyKey 才能跑），但已不是必需 |

---

## 9. 已知限制（写进 README）

- **便携模式 = 全键盘注入**：反作弊、过滤注入输入的软件、部分密码/安全控件可能拒绝输入。这是它相对驱动后端最本质的差异（驱动注入不带注入标记）。
- **与其它 LL hook 工具不共存**（AutoHotkey / espanso / 输入法的 hook 功能）：安装顺序决定效果，可能出现热键失灵或重复触发。
- **Win+L、Ctrl+Alt+Del、安全桌面（UAC 提示、登录界面）永远拦不到。**
- **多键盘共用同一套映射**（无 device 维度）；GUI 在便携模式下应隐藏/禁用 device 相关设置。
- **不提权运行时，提权窗口内映射静默不生效**（按键仍原生工作）；需要生效就以管理员身份启动 AnyKey。
- **鼠标移动不接管**（高频事件走"吞 + 重放"是性能灾难）：便携模式下鼠标**移动**原样透传，
  只有**按键 / 滚轮**会被拦截重放 —— 这与驱动后端完全一致（驱动也是纯移动直接转发、不入管道）。
- **鼠标按键/滚轮变成注入事件**：与键盘同理，过滤注入输入的软件可能不认。
- **紧急脱离快捷键在便携模式下由钩子实现**（2026-09-18 补）：同一套语义 —— LCtrl+Space+Esc 三键同时按住 → **立即结束引擎进程**，钩子随进程消失、键鼠恢复原生。实现在 `hook_input.rs` 键盘回调里、**先于一切判定**（对应驱动 `EMERGENCY COMBO CHECK` 那个 "highest priority" 的位置），因此引擎主线程死锁时仍然有效；只看物理键（注入事件不计入，否则 AnyKey 自己输出的这三个键会把引擎杀掉），只认左 Ctrl（E0 的右 Ctrl 不计入）。详见 §10。
- **没有「无人值守的自动恢复」**：心跳与 30s watchdog 已于 2026-09-18 **整体删除**（驱动 v0.5）—— 两个后端都没有了。键盘失灵时需要用户自己按上面的紧急键。
- **另外两条安全边界**：钩子随进程退出自动卸载、钩子回调绝不阻塞（队列满则原样放行）。⚠️ **存活对账当前只告警、不重装钩子**（见 §10 遗留项）。

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

### Step 2 —— 免驱动后端落地（提交 `78a5e39`）

**做了什么**

| 文件 | 改动 |
|---|---|
| 新增 `anykey-engine/src/hook_input.rs`（428 行） | 钩子安装 + 专用线程消息泵 + 有界通道 + 1:1 投影 + 存活对账 |
| 新增 `anykey-engine/src/sendinput_out.rs`（185 行） | 鼠标按键 / 滚轮 / 移动的 SendInput 实现（键盘复用 `emit.rs`） |
| `backend.rs` | 加 `LlHook(HookInput)` 变体；单设备桩 `enum_devices`；纯函数 `choose_backend` / `parse_backend_kind` |
| `main.rs` | `--backend=` 全 argv 扫描；§4.2 构造顺序（含驱动 open 重试与**引擎内回退**）；llhook 下强制 `perDevice=false`；心跳线程仅驱动；主循环存活对账；两处都失败退出码 1 |
| `Cargo.toml` | 加 `Win32_System_SystemInformation` feature（`GetTickCount`） |
| `lib.rs` | 声明两个新模块 |

**关键实现决定（记下来，免得以后回头猜）**

1. **钩子必须装在自己的线程上** —— 回调在"安装钩子的那条线程"上下发，且那条线程要有消息循环。所以钩子 + `MsgWaitForMultipleObjectsEx` 泵同住一条线程；线程退出 = 钩子失效，因此泵循环只在收到 `WM_QUIT` 时返回。
2. **回调绝不阻塞**：判定 → `try_send` → 立即返回。通道满（管道失速）时**放行该键**（直通降级）—— 宁可这次不映射，也绝不丢键。**不要**学 Kanata 的 `try_send_panic`（回调里 panic = 键盘卡死）。
3. **自注入过滤只认 `LLKHF_INJECTED` 一位，且必须放行**：本后端输出走 `SendInput`，会再次经过本钩子；吞掉它 = 自己的输出被自己吞掉 → 应用永远收不到键。**不打 `dwExtraInfo` 标记**（Kanata 也不打），少一层约定。
4. **E1 缺口用 `vkCode` 补**：`KBDLLHOOKSTRUCT` 没有 E1 位（只有对应 E0 的 `LLKHF_EXTENDED`），而 Pause 与 NumLock 扫描码都是 `0x45` → 投影会把 Pause 变成 NumLock。两者虚拟键码不同（Pause `0x13` / NumLock `0x90`），用 `vkCode` 兜住这一个特例。这是本后端**唯一**一处投影不完整的地方。
5. **llhook 模式下强制 `perDevice=false`**（只改内存副本、**不写回配置文件**）：否则用户配置里若 `perDevice=true` 且订阅的是真实设备（guid / VID:PID），伪设备 0 不在订阅集里 → 拿到的是**空映射（纯透传）** → 表现为"什么映射都不生效"。
6. **存活对账用 `GetLastInputInfo()`** —— 它是系统级、**不经过本钩子**的独立见证（此前实测验证过）。判定式："系统侧最近输入发生在本地最后一次回调之后，且已持续 >5s"。主循环每 3s 检查一次，不健康时限流 30s 告警（前台在提权窗口时也会命中，属预期，日志里写明了）。
7. **便携模式暂不接管鼠标输入**：同进程注册**键盘** Raw Input 会让 LL hook 立即停止被调用（附录 B 的实测），所以鼠标输入需要另想办法（独立进程或用鼠标钩子配 Raw Input —— 后者已实测不对称、机制上成立）。本期只做鼠标**输出**。
8. **两个新模块暂时与 `filter-driver` 同 `cfg`**，因为事件结构体（`AnyKeyInputEvent` 等）还定义在 `filter_driver.rs` 里。这是 Step 5 的清理项，**不代表它们"属于驱动"**。

**验证**

- `cargo test`：**122 项全绿**（16 组）= 原有 110 + 新增 12；**零警告**；`anykey-engine.exe` 正常产出。
- 新增的 12 项：6 项投影（普通键上/下、E0 扩展、PrintScreen 的 E0、Pause/NumLock 的 E1 区分、抬起位不被误置）+ 4 项后端选择与解析（`llhook` 恒不回退、`driver` 仅在不可用时回退、大小写与未知取值）+ 2 项坐标归一化（满量程与越界钳位）。

**【环境级】编译死锁（已固化进跨项目记忆）**

本机出现一个与代码无关的编译死锁，值得记下来，因为它会伪装成"编译很慢"：

- **现象**：在**工作区目录内**链接出可执行文件（`.exe` / `.dll`）时，产物已经写出，但 rustc 进程**永不退出** —— CPU 时间恒定不变（实测 16 秒采样 `UserModeTime`/`KernelModeTime` 完全不动），并留下未清理的临时文件（`rmetaXXXXXX`、`rustcXXXXXX`、`*.rcgu.o`）。
- **最小复现（决定性）**：同一条命令 `rustc --edition 2021 --out-dir <工作区内> main.rs` → 卡死；换成 `--out-dir %TEMP%\rtest` → **0.7 秒完成**。普通 bin、proc-macro crate、带不带 `-C prefer-dynamic` 都一样；**`--emit=metadata`（不链接）从不受影响**。
- **判定**：**位置相关**，不是代码、不是工具链。最可能是工作区被文件监听（IDE / 桌面端）持有新生成二进制文件的句柄，使 rustc 收尾的删除动作被阻塞 —— 与 `git rm` / `git switch` 在工作区内删文件导致进程被杀是**同一类**问题。
- **绕过**：把编译输出移出工作区 → `CARGO_TARGET_DIR=C:/Users/proje/AppData/Local/Temp/<name> cargo test`，随即一切正常（本次就是这么跑通的）。
- **诊断手法**：怀疑是"卡住"而不是"慢"时，隔 10 秒采样两次进程 CPU 时间，**数值不动 = 死锁**；再看命令行参数判断卡在哪个 crate。
- **两个易误判点**：① 本机 `cargo.exe` 常态有**两个**进程（`~/.cargo/bin/cargo.exe` 是 rustup **代理**，它会再起真正的 toolchain cargo），这是正常的，**不是**"两个构建在抢锁"；② 因为 `--emit=metadata` 不受影响，表现为**能 `cargo check` 但不能 `cargo build`**，很容易误判成编译慢。

**未做的验证（需要真机人工）**

- **最小映射端到端冒烟**：用真实配置以 `--backend=llhook` 起引擎，敲键盘确认映射生效、注入键到达应用。本步只做到"编译 + 单测通过"，**没有实机跑过钩子**。
- §8 的 9 项实测（Alt+Tab / AltGr / CapsLock 翻转 / 非美式布局 / 中文 IME / 延迟 / Pause 保真 / 锁屏唤醒 / 自动化测试通路）。

**下一步**：Step 3 —— 日志补齐（`requested` / `fallback` / **首次钩子回调** / `exit_reason`）。本步已打了 `backend requested`、`WARN fallback -> llhook`、`Backend: X`、存活对账，尚缺"首次收到回调"这一行（钩子"装上"不等于"收得到"）与退出原因的完整覆盖。

### Step 2b —— 补上鼠标拦截（提交 `b16c231`）

用户要求便携模式必须支持鼠标拦截与输出，且事件模型与现有驱动后端对齐。

**先核实前提（`tools/probe_llmouse.py`，五项全过）**

把 `WH_KEYBOARD_LL` 与 `WH_MOUSE_LL` 装在**同一条线程**上、共用同一个消息泵，然后自动注入
三类事件并核对。关键结论：**两个钩子可以共存**，键盘钩子不受鼠标钩子影响；鼠标钩子能看到
按键与移动；放行的移动与点击都正常到达目标。详见 §3.6。

**实现（对齐驱动的语义）**

- `hook_input.rs`：新增 `ms_hook_proc`；通道载荷由 `AnyKeyInputEvent` 改成
  `enum HookEvent { Key, Mouse }` —— **必须一条通道**，因为 `recv_timeout` 只能等一条，
  两条通道会让"等键盘"和"等鼠标"互相阻塞。这个语义与驱动"两个队列、同一次 poll 取走"一致。
- `project_mouse(msg, mouse_data) -> Option<(button_flags, button_data)>`（**纯函数 + 4 项单测**）：
  钩子消息 → 驱动的 ntddmou 位。两个只有读 `MSLLHOOKSTRUCT` 才知道的细节：
  ① **XBUTTON 的编号（1/2）藏在 `mouseData` 的高 16 位**，映射到驱动的 4/5 号键；
  ② **滚轮增量是有符号的**（`mouseData` 高 16 位按 `i16` 解释，下滚 −120）。
- **纯移动（`WM_MOUSEMOVE`）放行、不产生事件** —— 与驱动完全一致（`anykey_flt.c` 的注释原文
  "Pure movement (ButtonFlags==0): forward to system immediately, no queue"）。这既对齐了语义，
  也避开了"高频事件吞+重放"的性能灾难。
- **不注册任何 Raw Input**：便携模式没有设备维度，设备身份没有用处 —— 顺带完全避开了
  "注册键盘 Raw Input 会切断 LL hook"那个坑（§3 附录 B）。
- `flags` 恒为 `MOUSE_MOVE_RELATIVE`、`last_x/last_y` 恒为 0：按键/滚轮包里没有位移语义，
  驱动的那两个字段也不参与任何判定（引擎侧仅用于日志）。
- **鼠标钩子装不上 → 整体失败**（返回 `Err`，由 main 报错退出）：用户明确要鼠标拦截，
  静默降级会变成"以为能用"。
- `HookLiveness` 增加 `mouse_edges` / `mouse_moves`：日志里能把"鼠标钩子死了"和
  "键盘钩子死了"**分开**看，这直接决定了排查方向（例如提权窗口会让键盘路径静默、
  但鼠标按键计数仍在增长 → 说明钩子机制本身活着）。
- `sendinput_out.rs` 的 `XBUTTON1/2` 改成 `pub`：输入侧要从 `mouseData` 高 16 位**读**它、
  输出侧要往 `mouseData` 里**写**它 —— 一处定义、两处共用（单一真源）。

**验证**

- `cargo test`：**126 项全绿**（16 组）= 上一版的 122 + 4 项鼠标投影测试；零警告；exe 正常产出。
- 新增单测覆盖：三键按下/抬起、XBUTTON 编号来自高 16 位（含未知编号 → 放行）、
  滚轮有符号增量（±120）、纯移动不投影。
- **仍未做**：实机端到端冒烟（真敲键盘 + 真点鼠标，确认映射生效且注入不被打回）。

**下一步**：Step 3 —— 日志补齐（`requested` / `fallback` / **首次回调** / `exit_reason`）。

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

### Step 2c —— 实机冒烟测试通过：免驱动后端端到端可用（2026-09-17 22:31）

**测试方式**：`AnyKey/tools/smoke_llhook.py`（守卫脚本：到点强杀引擎 + 独立看门狗兜底），
以**调试构建 + 临时配置副本**启动 `--backend=llhook`，90 秒内按提示完成 4 步。
配置是复制到 `%TEMP%\anykey-smoke\` 的，用户的真配置与安装目录一个字未动。

**日志侧证据**（本次会话 4003 行）：

| 步骤 | 日志证据 | 结果 |
|---|---|---|
| ① 在记事本打 `hello world` | `in: DN/UP h,e,l,l,o…` 共 94 条输入；键盘重放 68 条 | ✅ |
| ② 按住空格 0.5s → 按 `q` | `[SEND] releaseKey physical=q` / `EmitUp: 1` → `send(FLT): DN 1` / `UP 1` | ✅ 输出 `1` |
| ③ 鼠标左键点击 + 滚轮 | 70 条 `mouse:` 输入；84 条 `send(FLT): MOUSE …`（`mouseleft` DN/UP、`wheeldown`×4、`wheelup`×5） | ✅ |
| ④ 按住 ` 1s → 按 `o` | `[LEADER] record phys=o logical={mouseright}` → `EmitMouseDown: mouseright` → `send(FLT): MOUSE DN/UP mouseright` | ✅ |

**用户目视确认**（四项全部正常）：打字无异常、出现 `1`、鼠标点击与滚轮正常、光标处弹出右键菜单。

**顺带验证到的**：
- 两后端输出**逐条同形**：llhook 的 `send(FLT): DN/UP <键名> dev=0` 与驱动后端一致；
  鼠标输入 `flg=0x00(rel) dx=0 dy=0 (raw x=0 y=0)` 与驱动给引擎的事件完全一致。
- **0 条 WARN / 0 条 ERROR / 0 条发送失败**。
- 存活对账误报已修（见下）：上一轮 90 秒内会打 `WARN … 键盘钩子已 23031ms 无回调`，本轮没有。

**本轮修掉的真 bug**（冒烟测试的主要收获）：
第 1 轮出现 `WARN llhook: 系统侧有输入但键盘钩子已 23031ms 无回调 (kb_callbacks=42 mouse_edges=0 mouse_moves=722)`：
根因是 `last_cb_tick` **只在键盘入队成功时更新**，而 `GetLastInputInfo` 对**鼠标活动**也推进 ——
于是"只动鼠标、不打字"被误判成"钩子被摘除"。修法：**两个钩子都在回调入口就更新心跳**
（含鼠标移动的透传路径），此后 `kb_callbacks` 仅是统计量、不再参与判定。

**两个流程教训（已写进 `windows-input-chain-verify` 技能）**：
1. 提示条**只把文字写在窗口标题**（900px 窄条）→ 长句被截断，用户看不到指令。
   要么把指令放进聊天消息，要么用 GDI 画在客户区；本轮采用"缩短标题 + 完整指令放消息里"。
2. **需要人配合的测试，启动前必须先用提问模式确认用户已在电脑前**。
   本轮有一次（`--plan fn3mouse`）就是默默启动、用户不在场，整轮 `kb_callbacks=0` 白跑一次。

---

### Step 3 —— 日志补齐：首次钩子回调 + 统一 exit_reason（提交 `ef59c9c`，全自动核对通过）

**改动**（`main.rs` + `hook_input.rs`，+65/−21）：

| 新增 | 位置 | 说明 |
|---|---|---|
| `log_exit(reason, code)` | `main.rs`（嵌套函数） | 统一退出原因记录，**模态 msgbox 之前**落盘（msgbox 会把进程卡住） |
| `exit_reason = …` | 配置读失败 / 配置非法 / 两个后端都失败 / `poll_all_failed` | 覆盖全部已日志化的退出路径 |
| `===== engine stop <ts> (backend=…) =====` | 主循环之后 | 收尾横幅 |
| `llhook: first hook callback received (…)` | 主循环（每次迭代判一次，**只打一行**） | **装上 ≠ 收得到**；绝不在回调里写日志（回调必须极快） |
| `HookLiveness.since_install_ms` | `hook_input.rs` | 让上面那行能报"安装后多久收到第一次回调" |

**核对方式：`tools/check_logging_step3.py`（全自动，不需要人按键）**。
能自动化的关键：`mouse_edges` 在钩子回调里**先于**"是否注入"的判断自增，所以一次
`SendInput` 注入的滚轮就能证明"钩子装上后确实收到了回调"；注入前把光标移到**脚本自己的置顶窗口**上、
用完立刻还原位置，不碰用户任何窗口。实测结果：

```
启动横幅 argv      OK    argv = [ ... "--backend=llhook" ]
请求的后端          OK    backend requested = llhook (来源: argv)
生效后端           OK    Backend: llhook
钩子安装           OK    llhook: SetWindowsHookExW(WH_KEYBOARD_LL + WH_MOUSE_LL) -> OK
首次钩子回调         OK    llhook: first hook callback received (kb_callbacks=0 mouse_edges=1, 4047ms after install)
存活对账           OK    llhook: liveness kb_callbacks=0 mouse_edges=0 mouse_moves=0 …
强杀读法           OK    日志里无 exit_reason / engine stop（taskkill 后确实两行都没有）
exit_reason 行     OK    exit_reason = config_read_failed
=> 全部通过
```

**这一轮修掉的两个小问题**：
1. `exit_reason` 的赋值在 `msgbox` **之后** → 弹窗是模态的，进程会卡在弹窗上，日志可能永远写不出。
   改成先落盘再弹窗。（"失败路径必须在退出前写盘"这条硬要求。）
2. `KEY_E0` 上面有**重复的 `#[cfg(feature = "filter-driver")]`**，顺手清掉。

**踩坑记录**：
- **`SendInput` 不能传 `byref(数组)`**：`ctypes` 报 `expected LP_INPUT instance instead of pointer to INPUT_Array_1`。
  正确写法是**直接传数组**（`user32.SendInput(1, arr, sizeof(INPUT))`），`ctypes` 会自动转成 `POINTER(INPUT)`。
  同一仓库里的 `probe_llmouse.py` 一直是对的写法，照抄即可。
- **测试脚本自己崩了也必须保证恢复**：第一版 `finally` 只还原光标、没杀引擎，脚本在注入处抛异常后，
  引擎被留在了系统里继续吞键。已改成 `SPAWNED` 列表 + `finally` 里无条件全部 `taskkill` 并复查残留。
  （这条同时回写进了 `windows-input-chain-verify` 技能。）
- **Python 文本模式写 `.rs` 会把行尾变成 CRLF**：`open(p,'w')` 默认 `newline=None`，在 Windows 上把 `
`
  写成 `

`。仓库里 `.rs` 是 LF，于是 `git status` 出现"幽灵修改"。写完必须核对 CRLF 计数并归一化
  （`raw.replace(b'\r\n', b'\n')`，字节数应**减少**）。

**验证**：`cargo test` **126 项全绿、零警告**。

---


### 2026-09-17 22:50　Step 4 动手前：GUI 源码完整快照（第二层保险）

- 用户提出"改 GUI 之前也要做快照"——第一轮快照（`backups/20260917_portable_backend/`）只覆盖了
  `gui/main.py`，`gui/` 其余 6 个文件与整个 `lib/` 都没覆盖，是真实缺口。
- 新增 `backups/20260917_gui_before_portable_backend/`：**12 个文件 / 321.6 KB**
  （`gui/` 7 个 + `lib/` 4 个 + `requirements.txt`）。`MANIFEST.txt` 记录 HEAD `f8240f43`、分支、
  当时的 `git status`、以及**每个文件的 sha256**；拷贝后逐文件哈希比对，**12/12 通过**。
- 事实核对（避免高估这层快照的用途）：`gui/` 与 `lib/` 那 11 个文件**全部已被 git 跟踪**，
  且快照时工作区干净 —— 所以这一层防的是**工具层 / git 事故**（本机 `git switch` 曾两次让
  `anykey-engine/src/` 整棵目录消失），而不是防手误。恢复优先级：
  `git restore --source=<sha> --worktree -- <具体路径>`（**必须按路径，不要 `.`**）→ 快照原样拷回。
- 顺带记一个仓库根的垃圾文件：`nul`（0 字节）。是 shell 把 `> nul` 当成重定向造出来的产物，
  不在 git 里、也不影响构建；因为是 Windows 保留设备名，常规删除方式不一定管用 —— **别去动它**。

### Step 4 —— 托盘传参 + GUI 开关（提交 `43b911f` / `83e490f`，全链验证通过）

**改动五处**：

| 文件 | 改动 |
|---|---|
| `anykey-tray/src/config.rs`（新） | `load_bool` / `load_backend` + 纯函数 `normalize_backend`；**每次调用都重读文件、不缓存**（写者是 GUI；缓存在托盘启动时会出现「GUI 改了 → 重载 → 还是旧值」） |
| `anykey-tray/src/engine.rs` | `start()` 里追加 `--backend=<config::load_backend(...)>` |
| `anykey-tray/src/app.rs` | `load_debug_flag` 改为复用 `config::load_bool`（去掉第二处 `serde_json` 取值） |
| `lib/driver.py` | 新增 `probe_driver()`：只读探测，返回 `(available, code)`，code ∈ `ok` / `not_installed` / `file_missing` / `not_loaded`（服务名 `anykey_flt`；读 `HKLM` 下服务的 `ImagePath` 并检查 .sys 是否存在） |
| `gui/main.py` | 侧栏新增「运行模式」小节（开关 + 说明行）；`_device_ui_enabled()` 作**唯一判据**；`_toggle_per_device` 拆成「重画 + 落盘」；`_refresh_devices` 末尾重探驱动；`_collect_cfg` 写 `backend` |

**为什么探测要分四种**：未安装 / 文件缺失 / 未加载 的**处理动作完全不同**
（运行 `安装驱动.bat` / 重装 / 检查 testsigning 并重启），只说「不可用」等于没说。

**三条验证（都不需要人参与）**：

1. `tools/probe_backend_switch.py`（假 self + 真 CTk 控件）→ `mismatches = 0`、`verdict = pass`。
   8 组真值表 + 控件 `state` + 文案归类 + 变量未被改写；期望值由**独立表达式**算出（不调用被测函数），
   所以被测代码写错会真的报 MISMATCH。输出是「期望 vs 实际」对照表。
2. `tools/smoke_backend_switch_e2e.py`（**真 AnyKeyApp + 临时配置**）→ 4/4 通过、`verdict = pass`：

   | 场景 | 期望 backend | 实际 | 「设备独立设置」开关 |
   |---|---|---|---|
   | 驱动可用 + 便携 OFF | driver | driver | normal |
   | 驱动可用 + 便携 ON | llhook | llhook | disabled（perDevice 仍保留 True）|
   | 驱动不可用 + OFF（存储 llhook）| **llhook（保留）** | llhook | disabled |
   | 驱动不可用 + ON（存储 driver）| **driver（保留）** | driver | disabled |

   同时 md5 证明**用户真实 `anykey_config.json` 未被改动**（前后都是 `64865729b2ff`）。
3. `tools/check_tray_backend_arg.py`（隔离沙箱：托盘/引擎副本 + **最小空映射配置**）→ 两轮 OK：
   配置 `backend=llhook` → 引擎 argv 出现 `--backend=llhook`；配置**缺该字段** → `--backend=driver`。
   沙箱用空映射，所以那 2×5 秒里键盘是「吞掉再原样重放」，不会改掉任何按键；跑完复查进程残留为 0。

**踩坑**：
- 托盘 `cargo test` **不产出 exe**（只出测试二进制）→ 验证参数传递前必须先 `cargo build`。
- PowerShell 里把两个 Tk 脚本串在一条命令里，第一个结束后会连带掐掉后面的 —— **一条命令只跑一个 GUI 脚本**
  （`anykey-gui-verify` 技能里记过，这次又撞了一次）。
- 探针第一版就崩：假 self 少设 `_driver_reason`。**探针自身也要按「期望值独立算出」来写**，
  否则产品代码一改字段名，炸的是探针而不是断言。
- 写补丁脚本时在双引号字符串里混进 ASCII 双引号 → SyntaxError。**中文正文里的强调一律用「」**。
- **在 bash 的双引号字符串里写反引号 → 被当成命令替换**：用 `python -c "..."` 改文档里的「提交 `abc1234`」时，反引号先被 shell 执行、内容被替换成空，结果提交号静默消失（文件写进去了、但内容错）。**改含反引号/`$` 的文本一律走补丁脚本文件，不要用 `-c` 内联**。

**备份**：改 GUI 前的完整快照见上面 22:50 那条记录（12 个文件、sha256 核对通过）。

**下一步**：Step 5（收尾解耦：四个事件结构体搬出 `filter_driver.rs`、`TapSI/DownSI/UpSI` 通道合流）、
Step 6（打包便携 ZIP + README 能力对照表）。

### Step 5 —— 收尾解耦：事件负载搬进 `src/events.rs` + 摘掉说谎的 cfg（提交 `18447ad`）

**做了什么**

| 项 | 结果 |
|---|---|
| 新增 `src/events.rs`（216 行，**无 cfg**） | 四个事件结构体 + 设备清单负载 + `ANYKEY_KEY_*` / `MOUSE_*` / `ANYKEY_DEV_FLAG_*` + `MouseEventTranslator` —— **按行从 `filter_driver.rs` 抽取**（不手抄：`#[repr(C)]` 注释与常量值一律原样搬） |
| `src/filter_driver.rs` | 763 → 585 行；只留驱动协议；加 `use crate::events::{…}`；`ANYKEY_FLAG_DEVICE_CHANGED` 留在 `AnyKeyDriverStatus` 旁（它是**状态位**，不该混进设备标志）；删掉已无用的 `use std::collections::HashMap` |
| `hook_input.rs` / `sendinput_out.rs` | import 改指 `events`；`lib.rs` **摘掉 cfg** |
| `app_sensor.rs` | 摘掉 4 处 `cfg(feature = "filter-driver")` 并删掉返回 `None` 的桩 —— 它只用 `SetWinEventHook` + `GetForegroundWindow`，与后端本来就无关 |
| `emit.rs` / `registry.rs` / `backend.rs` / `main.rs` | import 改指 `events`；`main.rs` 局部 `KEY_E0/KEY_E1` 删除，改用 `ANYKEY_KEY_E0/E1`（不再有两份位定义） |
| `state.rs` | `current_device: 1` → `0`（= public.h 的「默认设备」；原值 1 是设备从 1 起编号时代的残留。它是占位值，真实 emit 之前必被输入事件覆盖） |
| `commit.rs` | `emit_layer_act/deact` 里硬编码的 `1` → `self.current_device`；`tests/pipeline_test.rs` 7 处期望随之 1 → 0 |
| 7 个 `examples/` + 2 个 `tests/` | import 改指 `events`（`FilterDriver` 仍从 `filter_driver` 取） |

**验证**：`cargo test` **126 项全绿、零警告**（16 组）；`cargo build --examples` 零警告；全仓 `filter_driver::` 引用只剩 `FilterDriver`（10 处）+ `ANYKEY_FLAG_DEVICE_CHANGED`（1 处）。改动 22 文件 +287/−242（净减的是搬迁：`filter_driver.rs` −178，`events.rs` +214）。

**刻意没做的两件事**（都不是「忘了」，已写进 §5.0 / §5.4）：
1. **不合流 `TapSI/DownSI/UpSI`** —— 便携模式下两者确实都走 SendInput，但日志标签是 §6.3 诊断配方的一部分，`Fence` 时长又是 commit 的决策；收益只是少一段重复代码。
2. **不加 `llhook-backend` feature、不做 cfg 分叉** —— 没有这样的构建目标，`main` / `registry` 仍要求特性；本步目标是「cfg 不说谎」，已达成。

**⚠️ 又踩了一次同一个坑**：改注释时用内联 `python -c "…"`，字符串里含反引号 → 被 bash 当**命令替换**执行（`Broker: command not found`），替换结果为空、断言才拦住。教训重申（跨项目记忆里已有）：**凡改动文本含反引号 / `$` / `!`，一律先写补丁文件再执行**。同类：`git commit -F -` 的 heredoc 也会被 shell 包装层的 eval 弄坏（消息里的双引号）→ **提交消息写文件后 `git commit -F <file>`**。
### Step 6（部分）—— 回答「llhook 到底测了什么」，并把缺口补上（提交 `bfd4224`）

**起因**：用户问「驱动后端完全没变吧？测试还是用驱动后端吗？llhook 有没有真的通了 的测试？」

**核实结论（读代码得出，不是推测）**

| 问题 | 答案 |
|---|---|
| 驱动后端变了吗 | 行为未变。Step 5 的语义性改动只有 4 处：`KEY_E0/E1` 换成 `ANYKEY_KEY_E0/E1`（同值改名）、`emit_layer_*` 的硬编码 `1` → `self.current_device`、`current_device` 初值 `1` → `0`、以及搬迁本身。前两处 main 侧不读该字段；初值那处只影响「引擎启动后、**任何输入之前**就发生 app 切换」这一瞬态，而 `get_mapping(0,app)` 与 `get_mapping(1,app)` 走完 `build_multi_device_contexts` → `apply_app_override` 后结果相同（两者都查不到设备覆盖 → 落到全局 base 映射），也不会 panic（`subscribed=[0]` 时 `contexts.get(&0)` 必然存在） |
| `cargo test` 里的测试用哪个后端 | **都不用**。13 个 `tests/*.rs` 全部直接驱动 `PipelineState`（`key_down`/`key_up`），不构造任何后端。唯一碰驱动的是 8 个 `examples/test_*.rs` —— 手工硬件探针，`cargo test` 不编译它们（改过 example 要单独 `cargo build --examples`） |
| llhook 有哪些测试 | 三层：① 16 个单元测试（`project` 7 + `project_mouse` 3 + `sendinput_out` 坐标归一化 2 + `backend` 取值规则 4）；② `tools/check_logging_step3.py`（全自动，起真引擎跑 `--backend=llhook`，核对日志六行 + 强杀读法，靠"注入一次滚轮"证明钩子收到回调）；③ `tools/smoke_llhook.py`（**需人工按键**，唯一覆盖完整链路） |
| **缺口** | 钩子 → 回调 → 通道 → `poll_all` → 管道 → 输出 这条**接线**没有自动化覆盖。第 ①层只测纯函数，第 ②层不碰映射，第 ③层要人 |

**做了什么**

1. `hook_input.rs`：把「入队」与「吞键」拆开（原来焊在 `swallow` 一个开关上，A2 决策），
   得到 `capture` / `swallow` / `accept_injected` 三个开关 + 纯函数
   `decide() -> {Pass, Mirror, Swallow}`；`HookOptions::production()` / `::for_test()` 两个构造函数；
   `install_with()`，`install()` 变成生产简写；`Drop` 连 `capture` 一起清（退役时不再往没人读的通道灌）。
   **判定的正确性在这里是要命的**（吞错 = 键盘失灵），所以它拿到 4 个真值表单测，而不是埋在
   `extern "system"` 回调里。
2. 新增 `tests/llhook_backend_test.rs`（一个 test fn —— `SHARED` 是 `OnceLock`，同进程只能装一份钩子）：
   装钩子（`for_test()`，**只观察**）→ 注入 F13 down/up → 断言投影结果 → 注入滚轮 → 断言进同一条通道
   → `liveness()` 非零 → 把拿到的事件喂进 `PipelineState` → 断言 `emit_log` 里有映射后的键
   → `send_output` 发一个键 → **断言它又穿回我们的钩子**（这条正是生产必须过滤注入事件的理由）。
   用 F13/F14 是因为 Windows 上没有任何应用响应它们 → 注入不会在任何人窗口里留字符。

**验证**：`cargo test` **131 项全绿、零警告**（17 组 = 原 126 + 4 项 `decide` 真值表 + 1 项接线测试）。
**在 AnyKey 正跑着（驱动处于拦截态）的情况下也通过** —— 顺带实测到：SendInput 注入的键不经过驱动，
所以驱动拦截不影响它。

**过程中踩的三个坑（都写进测试注释了）**
1. `poll_all` 会**把整批取走**，"轮询到第一个匹配就返回"会丢掉同批的其余事件 —— 第一版就这样丢了 key-up。
   改成"收集一个时间窗再按扫描码筛"。
2. `commit.rs::is_valid_key_name()` 要求 `{X}` 多字符时必须在 scancode / 鼠标键名表里，
   而两张表只到 `f12` —— 输出写成 `{f14}` 会**被静默丢弃**（映射建好后 resolver 拒绝输出，
   `emit_log` 为空、没有任何报错）。测试改用 `{lshift}`（合法且单独按一下无任何副作用）。
   ⚠️ 顺带发现一个**用户侧隐患**：非法输出名只在 `--debug` 日志里留一行 `UNKNOWN KEY: {…}`，
   平时完全静默。建议后续在配置加载时校验一次并告警（或 GUI 保存时校验）。
3. 「驱动注入无标记键」（原 §8 计划）这条前提仍未实测 —— 探针 `tools/probe_drv_inject_to_hook.py`
   已写好，但它要求驱动**不处于拦截态**（即先退出 AnyKey），而用户当时正在用 AnyKey，
   探针正确地拒绝了执行。现在自动化不再依赖这条路径，所以它降级为「可选复核」。

### Step 6（完成）—— 打包策略定案 + README 能力对照表（2026-09-18）

**用户决策：不做两个发布包。**「发布包里面带着完整的 anykey、驱动、安装脚本。用户没安装驱动那就直接用 anykey 也行。」

⟹ 原计划的「便携 ZIP（不含 `anykeyFilterDriver/` 与安装脚本）」**取消**。核实 `build/build_release_package.py`（120 行）：它只组装 `anykey/` + `anykeyFilterDriver/` + `安装驱动.bat` 这一种结构，**已符合要求、无需改动**。理由自洽：没装驱动的用户直接跑 `anykey/anykey-gui.exe`，引擎按 `backend` 字段（或自动回退）走便携模式 —— 一条发布链覆盖两种运行方式。

**README 能力对照表**（中英同步，各 +48/−4；改前 `llhook` 在两份 README 里**零命中**）

| 位置 | 补的内容 |
|---|---|
| 新增 `## 两种运行模式`（置于「关于测试模式」之后，**不占编号**故不影响后续章节号与既有锚点） | 10 行能力对照表：前置条件 / 键位映射 / 鼠标 / 应用覆盖 / **多设备（便携 ❌）** / 提权窗口（便携 ❌）/ 与 AHK·espanso 共存（便携 ❌）/ 拒注入软件（便携 ⚠️）/ 内核级紧急脱离（便携 ❌）/ 延迟；另附「怎么选」（GUI 开关 + `backend` 字段 + `--backend=` 强制）与两条代价 |
| `## 1. 核心功能特性` | 新增「免驱便携模式」一条 |
| `## 2. 安装方法` | 提示：步骤 1–3 可整体跳过，直接运行 `anykey/anykey-gui.exe` |
| `### 4.1 失控安全防护` | 表格下加注：后两层是驱动内建的，便携模式不可用 |
| `#### 4.3.5 设备设置` | 加一条：便携模式下本节全不可用，**设置内容不会被清空** |
| `## 6. 文件架构` | `src/` 补 `backend.rs` / `hook_input.rs` / `sendinput_out.rs` / `events.rs`；测试数 109 → 131 |
| `## 8. 测试` | 中文 109 → 131，英文 73 → 131（**两版长期没跟上测试增长**） |
| `## 9. 安全说明` | 补便携模式段落 |
| `## 10. 文档` | 加本文档链接 |

**为写准这几段而专门核实的事实**（此前只有印象、没有依据）：

1. **心跳线程在便携模式下直接退出**：`main.rs` 打开 `\\.\AnyKeyFlt` 失败即打 `HEARTBEAT: …` 并 `return`。（⚠️ 这是**当时**的事实；v0.5 已把心跳与 watchdog 整体删除，见下方 v0.5 记录。）
2. **紧急脱离快捷键（LCtrl+Space+Esc）不可用于便携模式**：它实现在驱动的 `ServiceCallback`（DISPATCH_LEVEL）。

**校验**：两版 README 行尾仍为纯 LF（CRLF=0）；文档内全部 `](#锚点)` 均可解析；对照表两版均 12 行、列数一致。

**顺带发现的两处「文档与实现不一致」**

1. ⚠️ **§5.1 ④ 写的「钩子已被摘 → 重装 + 记日志」只实现了一半**：`main.rs:769-810` 的存活对账只更新 `liveness_healthy` 状态机并**打日志告警**（`SUSPECT` / `callbacks resumed`），**没有任何重装钩子的代码**（全仓搜 `重装|reinstall` 零命中）。低层钩子被系统静默摘除（如回调超时）后 AnyKey 不会自愈 —— **列为遗留项**，是否实现待定（属罕见路径，用户重启引擎即可恢复）。
2. README 的测试项数长期未更新（中 109 / 英 73，实际 131）—— 已一并修正。

### Step 6 补 —— llhook 紧急退出组合键（2026-09-18，用户决策「只做 A」）

**起因**：用户问「llhook 没有紧急退出键，会不会因为错误配置之类的原因锁死键盘鼠标？」。核实结论是**会，而且它比驱动后端更需要兜底**：

- llhook 是"吞掉再重放"：物理键判定 `Swallow` → `try_send` 成功即 `return 1`；鼠标**纯移动放行**、**按键/滚轮同样被吞**；通道 `CHANNEL_CAP = 512`，**填满才降级直通**。
- **引擎主线程卡死** → 前 512 个键鼠事件被吞且永不输出（键盘丢键、鼠标点不动但光标能动），之后才直通恢复（映射已失效）。而 llhook **没有任何自动恢复**：心跳线程不启动（`:500-528` 只在 driver 后端 spawn）、存活对账只告警不重装、也没有内核侧紧急键。
- **配置错误**（引擎一切正常，但输出为空或写了非法键名被静默丢弃）→ 相关键永久消失。

**做了什么**（`hook_input.rs`）

1. 三个纯函数/常量：`emergency_bit(scan_code, extended) -> Option<u8>`、`emergency_after(mask, bit, is_up) -> u8`、`EMERG_TRIGGER = 0b111`。语义与驱动的 `EMERGENCY COMBO CHECK` 逐条对齐：LCtrl `0x1D` / Space `0x39` / Esc `0x01`，**只认非 E0 变体**（右 Ctrl 的 MakeCode 同样是 `0x1D`，但带 E0，不计入）。
2. 回调里插在 `decide()` **之前**（对应驱动 `EMERGENCY COMBO CHECK` 的 "highest priority" 位置）；状态是 `Shared.emergency: AtomicU8` 位掩码，只用原子 or/and —— **不加锁、不分配、不写日志**，因为这段必须能在主线程已死锁时照常工作。
3. **只看物理键**：带 `LLKHF_INJECTED` 的事件不参与判定。否则 AnyKey 自己输出 LCtrl/Space/Esc 时会把自己杀掉。（驱动天然如此 —— 它只看得见物理键。）
4. 触发 → `emergency_exit()`：`TerminateProcess` **立即结束进程**（`abort` 兜底）。**刻意不走 Drop** —— 主线程死锁时 `Drop` 根本不会执行；而进程一旦消失，Windows 会自动摘除本进程装的所有钩子，键鼠立刻恢复原生。这也顺带解释了为什么"停止是硬杀、可接受"这条既有结论在这里正好成立。
5. 5 个真值表单测：只认左变体 / 忽略其它键 / 三键同按才触发 / 非重叠按不触发 / 抬起只清自己那一位。⚠️ 真正的触发路径**不可测**（会把进程杀掉，且注入事件不参与判定，造不出物理按键）—— 只测纯函数。

**验证**：`cargo test` **136 项全绿、零警告**（131 + 5），17 个 suite。

**文档同步**：两份 README 的对照表行（`内核级紧急脱离快捷键 ❌` → `✅ 钩子层`）、4.1 表下注与正文、两种运行模式末段；测试项数 131 → 136（中英同步）。

**未做**（用户明确「只做 A」）：用户态看门狗线程（主循环超时 → 自动退出）。因此便携模式仍缺"无人值守的自动恢复"，需要用户按紧急键。

**已知副作用**：紧急键命中时进程被硬终止，**不会留下日志**（回调里不能做 I/O）。用户只能从托盘发现引擎已退出。

### v0.5 —— 删除心跳与 watchdog（2026-09-18，用户决定）

**决策依据**（先核实、后拍板）：心跳的**唯一消费者就是 watchdog**（引擎侧心跳线程只在 driver 后端 spawn、返回值直接丢弃；驱动侧 `IOCTL_ANYKEY_HEARTBEAT` 只做「刷新 `LastHeartbeat`」+「回填自检结构」）。watchdog 覆盖的是「整个进程连心跳线程都冻结」这一极罕见场景（实测从未触发），而紧急组合键在两种模式下都能覆盖"键盘失灵"。**代价明确**：不再有「无人值守的自动恢复」——但用户接受。

**改动清单**（6 个文件；全部走补丁脚本 `tools/patch_remove_heartbeat.py` + `..._2.py`，27 处编辑）

| 文件 | 删除/修改 |
|---|---|
| `sys/anykey_flt.c` | `SessionActive` / `LastHeartbeat` / `ANYKEY_HEARTBEAT_TIMEOUT_SECONDS` / 键盘与鼠标两处 watchdog 判断 / `IOCTL_ANYKEY_HEARTBEAT` handler / 初始化与 Cleanup 里的复位；`EmergencyShutdown` 头注释改为 "emergency stop"；启动横幅 `v0.2.0` → `v0.5.0`（**原本就与实际版本不符**，顺手修） |
| `sys/public.h` | 删 `IOCTL_ANYKEY_HEARTBEAT`(+6) / `ANYKEY_HEARTBEAT_RESPONSE` / `ANYKEY_STATE_*`；`ANYKEY_DRIVER_VERSION` → `0x00050000` |
| 两份 INF | `DriverVer` → `09/18/2026,0.5.0.0` |
| `src/filter_driver.rs` | 删 `IOCTL_ANYKEY_HEARTBEAT` / `AnyKeyHeartbeatResponse` / `ANYKEY_STATE_*` / `FilterDriver::heartbeat()` |
| `src/main.rs` | 删心跳线程块；llhook 启动日志措辞改为 "no driver heartbeat/watchdog" |
| `examples/` | 删 `test_heartbeat_idle.rs`、`test_heartbeat_watchdog.rs`（专测被删功能）；`test_filter_driver.rs` 两处注释改为不依赖 watchdog 的说法 |

**保留**：`AnyKey_EmergencyShutdown` —— 紧急组合键仍用它（关拦截 + flush 队列 + 注入 8 个修饰键 BREAK + 5 个鼠标键 UP）。

**验证**：`cargo test` **136 项全绿、零警告**（17 suite）+ `cargo build --examples` 零警告；驱动 `build_driver_release.py` **RC=0**（cl/link + 测试签名 + 部署 `deploy/anykey_flt.sys`，26.5 KiB）。

**踩的两个坑（补丁脚本的断言救了一次，另一次漏网）**

1. ✅ **断言先后拦下两次错误定位**：① `g_AnyKey.SessionActive = FALSE;` 后跟 `LastHeartbeat.QuadPart = 0;` 在 **DriverEntry 初始化**与 **EmergencyShutdown 末尾**各有一处（两处形式完全相同！）→ 只能先删 EmergencyShutdown 那处（靠 `// 5. Reset session state` 注释定位）再删 DriverEntry 那处；② `if planned == BackendKind::Driver {` 在 main.rs 里也有两处 → 改成"心跳块起始之后第一次出现"。**零副作用**（写盘统一在末尾）。
2. ⚠️ **跨行字符串替换必须连结束符一起换**：把 llhook 启动日志（`log!("…\` + 续行 `…");`）换成两行时**漏写了 `");`**，导致从该处起整个 `main.rs` 的引号配对错位 → **51 个语法错误**，且报错行散落在 765/944/971 等毫不相关的位置（`unterminated character literal`、`character literal may only contain one codepoint`），极具误导性。教训：**改跨行字符串后先单独 `cargo check` 拿到第一现场，别等全量测试**；判断依据是"错误数量远超改动量"就应怀疑字符串没闭合。

---



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
| 鼠标按设备识别 | **不对称已实测**：钩子吞掉鼠标移动后 RawInput **仍照常投递**（光标冻结 Δ=0 而 RawInput 收到 327 包）→ 机制上成立，但便携模式没有设备维度，用不上；按键/滚轮拦截不走 Raw Input（见 §3.6） |

---

## 附录 B：探针索引（`AnyKey/tools/`，均保持未跟踪）

| 探针 | 证明了什么 |
|---|---|
| `probe_elev_hook2.py` | 未提权钩子收不到发往提权窗口的按键（`GetLastInputInfo` 独立见证 + 收尾自检排除钩子中途失效） |
| `probe_dual.py` / `probe_ahk_cross.py` | 钩子吞键 → 被吞的键不再产生 `WM_INPUT`（双进程对照 + AHK 交叉验证） |
| `probe_hook_combo.py` / `probe_hook_pump.py` | 同进程内注册键盘 Raw Input 会切断 LL hook 投递（与线程/顺序/窗口无关，可逆） |
| `probe_mouse_swallow.py` | 鼠标移动与键盘**不对称**（吞掉后 RawInput 仍投递） |
| `probe_llmouse.py` | **两个 LL hook 可在同进程/同线程共存**（键盘钩子不受鼠标钩子影响；放行的移动与点击正常到达）。判定移动时要看方向与量级，因为相对移动会被"指针加速"缩放 |
| `probe_layout_erase.py` / `probe_layout_raw.py` | 布局把 `_none_` 变成 `vk=0xFF` 而非丢弃；布局改写发生在 RawInput 分支之前 |
| `probe_kbdvsc.py` | 全量读 218 个布局的 `pusVSCtoVK`（判断"布局能否表达 X"） |
| `probe_nolegacy.py` | `RIDEV_NOLEGACY` 会连带砍掉 `SendInput` 重放 |
| `probe_tsf_profiles.py` | TSF 同一时刻只有一个 active profile |
| `smoke_llhook.py` | **便携后端端到端冒烟**（需人按提示操作）：键盘重放 / hold 层 / 鼠标拦截重放 / 键盘→鼠标输出。守卫三件套＝到点强杀 + 独立 DETACHED 看门狗 + 配置复制到 `%TEMP%`（不碰用户安装目录）。**启动前必须先用提问模式确认用户已在电脑前** |
| `check_logging_step3.py` | **全自动**核对日志行：正常启动的六行 + 强杀后"无 exit_reason"读法 + 配置读失败的 `exit_reason`。用"注入一次滚轮"触发钩子回调（`mouse_edges` 先于注入判定自增），并把光标临时移到脚本自己的窗口上再还原 |
| `doc_to_main.py` | 用 git 底层命令把文档提交到 main —— **本机 `git switch` 会毁工作区**（见 §10 事故记录），所以不切分支 |
| `probe_backend_switch.py` | 后端开关 × 设备栏联动的**判据回归**：8 组真值表 + 控件 `state` + 文案归类 + 变量未被改写。假 self + 真 CTk 控件，快、不出窗口，rc 0/1 可进 CI |
| `smoke_backend_switch_e2e.py` | **真 AnyKeyApp + 临时配置**：`backend` 字段的 4 组保存规则（含「驱动不可用时不覆盖意图」「灰 ≠ 清空」），并 md5 证明**用户真实配置未被改动** |
| `tests/llhook_backend_test.rs`（在仓内，非 tools/） | **便携后端的接线测试**（Step 6）：投影 → 通道 → `poll_all` → 管道映射 → 输出回流。全自动、不需驱动/管理员/人工，装钩子时 `swallow=false` 故不影响本机键盘 |
| `probe_drv_inject_to_hook.py` | 核实「驱动注入能否当便携后端的物理键」（设计 §8 的原计划前提）：对比驱动注入与 SendInput 注入在钩子处的 `INJECTED` 标志。**要求驱动不处于拦截态**（先退出 AnyKey），否则探针主动放弃执行。现已成为可选复核 |
| `check_tray_backend_arg.py` | **托盘 → 引擎的参数传递**：在 %TEMP% 造隔离沙箱（托盘/引擎副本 + 最小空映射配置），读引擎日志的 argv 行确认 `--backend=` 取值正确；两轮（llhook / 缺省） |
