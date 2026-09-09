# AnyKey 代码文件图谱（Code Map）

> 给读代码的人用的项目结构索引。AnyKey 是一个「键位重映射 + 运行时控制 + 系统级输入拦截」工具，
> 分为四大运行模组（GUI / Tray / Engine / Driver）加一层共享库（Lib）与构建/测试外设。

---

## 0. 顶层布局

```
AnyKey/
├── gui/                      # 模组 1：配置生成前端（Python + CustomTkinter）
├── anykey-tray/              # 模组 2：运行控制 / 生命周期（Rust）
├── lib/                      # 共享层：配置 / IPC / 驱动封装（Python）
├── anykey-engine/            # 模组 3：Rust 引擎主功能（后端核心）
├── anykey-filter-driver/     # 模组 4：内核过滤驱动（C，WDK）
├── engines/rust/             # 引擎编译产物落盘位（构建产物，不入库）
├── assets/                   # 图标、帮助文档等资源
├── build/                    # 构建链：七步总控 + 各组件打包脚本
├── scripts/                  # 辅助生成脚本
├── tests/                    # Python 配置解析测试
├── docs/                     # 设计说明（DESIGN.md）与架构图 svg
├── CODEMAP.md                # 本文档
└── anykey_config.json        # 全局配置根（运行时生成/维护，GUI / Tray / Engine 共享）
```

依赖方向（单向，无循环）：
`GUI → Lib`　、`GUI → IPC ⇄ Tray`　、`Tray → 调度 Engine`　、`Engine → FilterDriver → 内核`。

---

## 1. GUI 模组（`gui/`）—— 配置生成器

纯前端，只读/写 `anykey_config.json`，**不含运行时逻辑**。

| 文件 | 职责 |
|------|------|
| `__init__.py` | 包导出 |
| `main.py` | CustomTkinter 主窗口、页签调度、事件循环；`_current_app` 状态 + 应用感知回调；调用 `lib.config` 读写配置；`_collect_combo_rows` 自动剥离 combo key1/key2 中的 `{}` 花括号 |
| `app_bar.py` | 应用感知顶栏独立组件：下拉（全局/已配置/运行中进程）+ 刷新/浏览/删除/导出/导入按钮；`psutil`+`win32gui` 枚举前台进程 |
| `components.py` | 可复用 UI 组件（按钮、卡片、表单绑定） |
| `layout.py` | 各配置区的布局定义（Combo / Layer / TapDance / Device 编辑器） |
| `dialogs.py` | 弹窗：设备选择、键名拾取、大编辑窗口（LargeInputDialog）、确认删除 |
| `scanner.py` | 枚举已接入设备 / 读取当前配置快照供编辑 |

> 与 Tray 的边界：GUI 通过 `lib.ipc.IpcClient`（TCP 127.0.0.1:19527）向 Tray 发命令，但**绝不自己起引擎**。

---

## 2. Tray 模组（`anykey-tray/`）—— 运行控制器

独立 Rust 进程（`anykey-tray.exe`），管理引擎生命周期，是 IPC 服务端。

| 文件 | 职责 |
|------|------|
| `Cargo.toml` / `Cargo.lock` | Rust 依赖与锁版本 |
| `src/main.rs` | 入口：单实例启动托盘 + IPC 服务线程，调度引擎进程 |
| `src/app.rs` | `Shared` 共享状态（运行/暂停/调试标志、引擎进程句柄） |
| `src/tray.rs` | 托盘菜单：打开主窗口 / 暂停·恢复 / 重载设置 / 开机自启 / 调试模式 / 退出；图标区分运行/暂停两态 |
| `src/engine.rs` | 引擎进程生命周期（启动 / 停止） |
| `src/ipc.rs` | IPC 服务端：TCP 127.0.0.1:19527，JSON 行协议（与 `lib/ipc.py` 完全兼容）；单连接模式，非阻塞 accept + 轮询 |
| `src/icon.rs` | 内嵌托盘图标资源 |
| `src/paths.rs` | exe 路径解析（配置文件 / 引擎 exe / GUI exe 定位） |
| `src/registry.rs` | 开机自启（注册表 Run 项读写） |

> 与 GUI 的边界：Tray **不编辑配置**，只接收 GUI 通过 IPC 发来的 `pause/resume/reload/status/quit` 并作用在引擎进程上。

---

## 3. Lib 共享层（`lib/`）—— GUI 与 Tray 的公共底座

| 文件 | 职责 |
|------|------|
| `__init__.py` | 包导出 |
| `config.py` | 配置加载/保存/校验；键名规范化（缩写→全名）；`_KEY_ALIAS_TO_FULL` 含旧鼠标键名→新规范名迁移（`lbutton→mouseleft` 等）；`_KEY_NORM_FALLBACK` 鼠标条目为自身映射；`normalize_layers_key_outputs` 是唯一规范化源头 |
| `ipc.py` | `IpcClient`（GUI 侧，连接 Tray）；Python 版 `IpcServer` 保留供测试（`build/test_tray_ipc.py`）；协议：TCP localhost:19527 + JSON 行协议，服务端实现在 `anykey-tray/src/ipc.rs` |
| `driver.py` | 用户态驱动调用封装：打开/查询/注入；与 `anykey-engine/src/filter_driver.rs` 共享 `public.h` 的 IOCTL 契约 |

> 关键铁律：`config.py` 的键名表与 `anykey-engine/src/util.rs` 的 `wrap_single_key_output` 必须逐字节对齐。

---

## 4. Engine 模组（`anykey-engine/`）—— Rust 后端核心

唯一后端，承载 Combo / TapDance / Layer / Leader / Defer 全部主要逻辑。
管道阶段（Phase0-7 / Up1-8+UpLeader）的调度与 wrapper 在 `pipeline.rs` 主干，
各系统核心逻辑拆分至 `pipeline/` 子模块（见 4.1 结构图）。

### 4.1 Rust 源码（`src/`）

| 文件 | 职责 |
|------|------|
| `lib.rs` | crate 根，导出全部子模块 |
| `main.rs` | 引擎可执行入口（`anykey-engine.exe <config.json> [--debug]`）；事件驱动主循环、键盘+鼠标、看门狗心跳 |
| `config.rs` | `Config` / `AppOverride` / `DeviceOverride` 结构 + serde 解析（combo / layers / tapDance / devices / subscribed_devices）；`AppOverride` 须带 `#[serde(rename_all = "camelCase")]`，否则 JSON `comboMap` 字段静默丢失 |
| `state.rs` | `PipelineState` + 各状态结构；**`mapping: Arc<DeviceMapping>` + `state: DeviceState`** — 管道直接字段访问；`current_device` 为输出目标设备；`current_domain` 为域标识；timer 调度（TimerEntry 带 Arc 快照） |
| `pipeline.rs` | **管道主干**：`key_down(key)`/`key_up(key)` 入口；`self.state`/`self.mapping` 直接字段访问；`key_down_inner`/`key_up_inner` 按序调度 Phase0-7 / Up1-8+UpLeader；定时器（reenter_up / fire_sleep_timer）；共享查询层（current_layer / is_switch_key 等） |
| `pipeline/combo.rs` | Combo 系统：匹配（match_combo_unified / try_match_combo / combo_lookup）、状态机（setup_combo_state / resolve_key_up）、打断（combo_interrupt_all）、超时（single_key_timeout / start_single_key_timeout）、清理（cleanup_released） |
| `pipeline/commit.rs` | Commit 系统：resolver（logical_key 解析）/ output / commit_stage（P7 + Up5 入口）；send_key / release_key（Down/Up 发送统一出口）+ record_emitted + is_valid_key_name；emit_* 发送通道 + activate/deactivate_layer |
| `pipeline/tap_dance.rs` | TapDance 状态机：tap_dance_down/up、set_td_state、hold/dt/dh 三个计时器、td_interrupt_all、flush_tapdone_as_tap；`TdKind` 六态 |
| `pipeline/defer.rs` | Defer 系统：延迟决策（try_defer_layer_interrupt）、force-hold（force_hold_execute / force_hold_switch_key / fire_switch）、waiting_stack（enter/remove_waiting_stack_as）、flush_deferred_entry |
| `pipeline/leader.rs` | Leader 系统：拦截+记录钩子（leader_intercept_record）、滑动超时、匹配（leader_match）、执行器（leader_execute）、循环捕获级联 |
| `emit.rs` | 底层输出原语：`key_name_to_scancode`（SendInput 回退表）/ `mouse_name_to_flags`（键名→FLT 按钮标志）/ `is_mouse_key_name`；鼠标键名已统一为驱动规范名（`mouseleft`/`mouseright`/`mousemiddle`/`mouseside1`/`mouseside2`），旧别名已删除 |
| `registry.rs` | 设备枚举（与 FilterDriver 共享）；`DeviceDescriptor` 列表 |
| `matcher.rs` | 设备匹配规则（GUID+VID+PID+HWID）；热插拔重匹配 |
| `runtime_builder.rs` | Mapping 构建 + **RuntimeManager v3**（`domains: HashMap<u32, DeviceState>` + `mappings: HashMap<(u32,String), Arc<DeviceMapping>>`）；`merge_combos` / `merge_layers` / `merge_leader` 合并；`build_combo_index`（combo 键名经 `norm_key` 规范化）；`apply_app_override`（四层覆盖链）；测试已分离至 `tests/runtime_builder_test.rs` |
| `app_sensor.rs` | SetWinEventHook 窗口类名侦测 + mpsc channel |
| `util.rs` | `wrap_single_key_output`、扫描码表、`is_layer_key`、`needs_e0` 等工具 |
| `filter_driver.rs` | `#[cfg(feature="filter-driver")]`：与内核驱动通讯（WAIT_INPUT / SET_INTERCEPT / 鼠标注入） |

管道拆分结构（`pipeline.rs` 主干 + `pipeline/` 五系统子模块，phase wrapper 留在主干做 flag/debug 转发，核心逻辑在各自模块；行数为约数）：

```
pipeline.rs            主干调度 + Phase0-7/Up1-8+UpLeader wrappers + 共享查询层（~630 行）
├── combo.rs           Combo 匹配/状态机/打断/超时/清理（~300 行）
├── commit.rs          resolver/output/commit_stage + send_key/release_key + emit_* 通道（~790 行）
├── tap_dance.rs       TD 状态机 + hold/dt/dh 计时器 + interrupt（~410 行）
├── defer.rs           延迟决策 + force-hold + waiting_stack（~160 行）
└── leader.rs          Leader 拦截/匹配/执行/级联（~280 行）
```

### 4.2 构建与数据

| 文件/目录 | 职责 |
|-----------|------|
| `Cargo.toml` / `Cargo.lock` | Rust 依赖与锁版本 |
| `examples/` | 配置样例 |
| `tests/` | `cargo test` 集成测试（合计全仓 109 项测试）：`scenario_test.rs`（golden 比对）+ `scenarios/` 场景集、`pipeline_test.rs`、`runtime_builder_test.rs`、`app_aware_test.rs`、`doubletap_hold_test.rs`、`hotplug_test.rs`、`leak_test.rs`、`reachability_test.rs`、`device_routing_test.rs`、`toggle_layer_test.rs`、`combo_mouse_correct_flow.rs`、`combo_mouse_merged_packet.rs`、`mouse_translator.rs`、`probe/` 探针数据 |
| `target/` | 编译产物（debug/release，不入库） |

---

## 5. Filter Driver 模组（`anykey-filter-driver/`）—— 系统通讯

内核级 UpperFilter（kbdclass / mouclass 下方），替代 Interception（无设备数量上限、支持热插拔/休眠）。

### 5.1 驱动源码（`sys/`）

| 文件 | 职责 |
|------|------|
| `public.h` | IOCTL 与 `ANYKEY_*` 结构体**唯一真值定义**（与 `filter_driver.rs` / `lib/driver.py` 三方对齐） |
| `anykey_flt.h` | 驱动内部头（设备扩展、回调原型） |
| `anykey_flt.c` | 过滤主逻辑：`KbFilter_ServiceCallback` / `MouFilter_ServiceCallback`；拦截态入队、透传捕获；含内核级紧急脱离快捷键（LCtrl+Space+Esc） |
| `rawpdo.c` | 原始 PDO 创建（设备枚举、即插即用） |
| `hello_flt.c` | Microsoft kbfiltr 样例基线 |

### 5.2 用户态与装载

| 文件/目录 | 职责 |
|-----------|------|
| `build_driver.bat` / `build_driver.ps1` | WDK 编译 + 测试签名 |
| `deploy/` | 发行目录：`Install_AnyKey_Filter.bat`（一键安装）/ `Uninstall_AnyKey_Filter.bat` + `_install_anykey_device.ps1` / `_uninstall.ps1`（实际安装逻辑）+ `anykey_flt.inf`（Keyboard）/ `anykey_flt_mouse.inf`（Mouse）+ `anykey_flt.sys` + `anykey_flt.cer` |
| `bin/`, `Release/`, `build.log` | 本地编译产物与日志（不入库） |

> INF 分两份：`anykey_flt.inf`（Keyboard class）+ `anykey_flt_mouse.inf`（Mouse class），同一 `.sys` 多引用。

---

## 6. 资源、构建与测试外设

| 路径 | 职责 |
|------|------|
| `assets/help.md`, `icon.ico`, `icon.png` | 帮助文档 + 托盘/窗口图标 |
| `build/build.bat` | 一键七步总控：驱动→引擎→GUI→托盘→组装→签名→release |
| `build/anykey.spec` | PyInstaller 打包规范（GUI → exe） |
| `build/build_driver_release.py` | 驱动构建包装（编译+签名+原子部署到 `deploy/`） |
| `build/build_engine_release.py` / `build_engine_release.bat` | Rust 引擎 release 构建 |
| `build/build_tray_release.py` | Rust 托盘 release 构建 |
| `build/build_release_package.py` | 发行包组装（`release\` 标准目录）+ `--zip <版本>` 压缩 |
| `build/test_tray_ipc.py` | Tray IPC 协议测试工具 |
| `engines/rust/` | 引擎 exe 落盘位置（Tray 启动目标，构建产物不入库） |
| `scripts/_gen_stacking_scenarios.py` | 生成 Leader/TapDance 堆叠测试场景 |
| `tests/` | Python 配置解析测试（`test_config.py` 等） |

---

## 7. 进程与数据流（运行期）

```
[anykey-gui.exe] ──IPC(cmd)──▶ [anykey-tray.exe] ──启动/调度──▶ [anykey-engine.exe]
   (lib.ipc                  (anykey-tray/src/              (Rust, 读 anykey_config.json)
    .IpcClient)               ipc.rs 服务端)                       │
                                                                    ▼
                                                          [Filter Driver .sys]
                                                           ↕ 内核输入队列
                                                          (键盘/鼠标设备)
```

- GUI 只写配置 + 透过 IPC 发命令，永不自己处理按键。
- Tray 是唯一「持有引擎进程」的角色：pause/resume/reload/quit 都作用在该进程。
- Engine 把所有按键决策转成 SendInput / FilterDriver 注入，经驱动回到系统设备队列。

---

## 8. 关键不变量（跨模组）

1. **配置根**：`anykey_config.json` 是 GUI/Tray/Engine 三端共享的唯一真相（与 exe 同目录）。
2. **键名规范化**只发生在 GUI（`lib/config.py`）；引擎仅作防御层。
3. **IOCTL/结构体三方对齐**：`public.h`（C）↔ `filter_driver.rs`（Rust）↔ `lib/driver.py`（Python），任何一侧改 IOCTL 功能号必须同步三处。
4. **进程边界**：GUI（Python）与 Tray（Rust）是两个独立进程，经 TCP localhost:19527（JSON 行协议）通信；Engine 是独立 Rust 进程，经 Filter Driver 与内核通讯。

---

## 9. 设备+应用感知运行时架构（v3）

### 9.1 核心概念

AnyKey 不再是"键盘映射器"，而是**输入上下文管理器**——一个物理环境（键鼠组合）中，
不同应用可能需要不同的映射规则，但输入状态（层、修饰键、combo 缓存）应跨设备共享。

为此 v3 将运行时拆成两个独立维度：

```
PipelineState
 ├── mapping: Arc<DeviceMapping>    隔离单位 = (device_id, 应用进程名)
 └── state:   DeviceState           隔离单位 = domain_id
```

- **mapping**：决定 "Space 是什么、A 键映射什么、combo/leader 规则"。
  按 `(device, app)` 索引——键盘+Chrome 与 鼠标+Chrome 各有独立 mapping。
- **state**：保存当前 layer 栈、modifier 状态、tap-hold 状态、defer 等待、combo 缓存。
  同一 domain 的键鼠**共享**这份 state——鼠标移动时可见键盘的 layer 状态。

### 9.2 RuntimeManager（`runtime_builder.rs`）

```rust
pub struct RuntimeManager {
    domains:  HashMap<u32, DeviceState>,                // domain_id → 共享状态
    mappings: HashMap<(u32, String), Arc<DeviceMapping>>, // (device,app) → 映射快照
}
```

方法：
- `get_mapping(config, device, app)` — 按 (device,app) 构建/缓存 `Arc<DeviceMapping>`。
  首次调用时从 config 构建（走 `build_multi_device_contexts` → `apply_app_override`），
  后续 Arc 复用，零 clone。
- `load_domain_state(domain_id)` — 加载域状态，不存在则建 `DeviceState::default()`。
- `save_domain_state(domain_id, state)` — 将状态归还管理器。

### 9.3 切换策略（仅边界，非每事件）

| 场景 | 操作 | 开销 |
|------|------|:--:|
| 同域 + 同 app，键盘→鼠标交替 | 只换 `pipeline.mapping = Arc<DeviceMapping>` | 零 clone |
| 同域 + 切 app（如 Chrome→PS） | 还旧 state → 设新 Arc mapping | state clone 一次 |
| 跨域（如物理键盘→虚拟键盘） | save 旧 state → load 新 state + 新 mapping | state clone 一次 |

PipelineState 长期持有 `mapping` + `state`，不经过 manager 中转。
只有 domain 或 app 变化时，才与 manager 交互。

### 9.4 计时器上下文快照

每个 `TimerEntry` 携带一个 `Arc<DeviceMapping>` 快照——冻结调度时的映射上下文。
计时器触发时临时 swap 到快照 mapping 处理，处理完还原当前 mapping。

这解决了 sleep 宏跨 app 后半段丢失、hold timer 切 app 后用错误规则解释等问题。

### 9.5 覆盖链（四层）

```
Layer 0: config 全局（tapDance / comboMap / leader）
Layer 1: config.appAware.apps[app]              ← 全局 app fallback
Layer 2: config.devices[guid]                    ← 设备覆盖
Layer 3: config.devices[guid].apps[app]          ← 设备+app 专属
```

四段均用 `merge_*` 做条目级覆盖（同身份覆盖，不同身份新增）。
全局 tapDance timing（holdTerm / doubleTapTerm / doubleHoldTerm）不可被 device 或 app 覆盖。
`apply_app_override` 在 `runtime_builder.rs` 中实现四层合并链；引擎仅在 domain/app 切换时调用，不在输入路径上构建。

### 9.6 应用侦测（`app_sensor.rs`）

使用 `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` 监听前台窗口切换，
通过 `QueryFullProcessImageNameW` 获取进程名（如 `chrome.exe`），
经 mpsc channel 异步发送给主循环。主循环仅在进程名变化时触发 mapping 切换。
