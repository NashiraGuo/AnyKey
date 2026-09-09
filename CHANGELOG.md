# AnyKey v1.0.0 — 更新日志

## 架构变更（重大）

- **输入后端收敛为单一内核过滤驱动**：彻底移除 Interception 依赖，自研 Windows UpperFilter 驱动（键盘 + 鼠标双设备类）成为唯一后端；引擎经 Filter Driver 与内核通讯。
- **Runtime v3**：`mapping(device, app)` 与 `state(domain)` 解耦；`TimerEntry` 携带 `Arc<DeviceMapping>` 快照，计时器跨设备 / 应用切换不串规则。
- **四层配置覆盖链**：全局 → `appAware.apps[app]` → `devices[guid]` → `devices[guid].apps[app]`，条目级合并（全局 TD 阈值不可覆盖）。

## 新功能

- **ToggleLayer `{tnX}`**：固定切换层，keyup 静默不翻回；层间叠加压栈，`{tn0}` 清空整栈切回 base；幂等。
- **层激活虚拟键三态**：`{fnX}` 连发 / `{bnX}` 吞重复 / `{tnX}` 固定切换。
- **内核级紧急脱离快捷键**：`LCtrl + Space + Esc` 三键同时按住，于驱动 `ServiceCallback`（DISPATCH_LEVEL）内核层检测触发，旁路引擎直接关停（关拦截 / 清队列 / 释放修饰键 / 重置状态）—— 防主线程死锁锁死键盘。
- **失控安全网三层（互不共享检测信号）**：Session（引擎进程退出立即关拦截）/ Heartbeat（30s 无心跳 IOCTL 兜底）/ 紧急脱离（主线程 hang 但心跳正常时手动脱离）。
- **应用感知双机制**：`SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` 事件驱动 + 独立 hook 线程 **500ms 轮询看门狗**（`GetForegroundWindow` 兜底比对，防 WinEvent 通知丢失）；事件回调仅置脏标记，按键热路径不查窗口。
- **宏语法 `{KeyName N}`**：把指定键 tap N 次（如 `{a 2}` = 按 a 两次；`hira@126.com{left 12}` = 先发文本再 Left tap 12 次）。
- **鼠标 5 键入管道**：左 / 右 / 中 / X1 / X2 支持 TD / Combo / 层切换；滚轮与移动透传；`MouseMove(x, y)` 相对移动宏。

## 文档

- README 重写为 GitHub 开源版（去历史化、补驱动 Test Signing 签名限制章节、配置示例对齐引擎 `config.rs` 实际结构）。
- 功能介绍合并入 README。

## 测试

- `cargo test` 约 73 项（单元测试 + 场景测试），集成测试归入 `tests/` 目录。

## 发布

- 引擎 / 过滤驱动 / GUI 独立打包，版本不强制对齐，仅依赖稳定 IPC。
- 驱动**无微软签名证书、未 WHQL 认证**，仅可在 Windows Test Signing 模式加载（`bcdedit /set testsigning on`）。

---

# AnyKey v0.9.0 — 更新日志

## 新功能

### Rust 原生引擎（anykey-engine）
- 基于 Interception 驱动的 Rust 原生引擎，替代 AHK SendInput 模式
- 性能：release 构建仅 4.3MB，毫秒级响应
- 支持：键盘 + 鼠标 + 媒体键全覆盖
- 调试开关与 GUI 托盘菜单联动（`--debug` 传参）
- 日志写入 GUI 同级目录（`anykey_engine.log`）

### 鼠标支持
- 5 个鼠标按键（左/右/中/X1/X2）全部入管道，支持 TD / Combo / 层切换
- 滚轮透传，鼠标移动透传
- `MouseMove(x, y)` 宏：相对移动输出（需在 hold/tap/dt/dh 中配置）
- 鼠标键名规范化：`MouseLeft → lbutton`，`MouseSide1 → xbutton1`

### 宏增强
- `RUN:`：改用 `ShellExecuteW`，支持文件夹/URL/任意文件，无 cmd 弹窗
- `{Sleep N}`：通过 SleepTimer 实现真正的非阻塞延迟

### 设备白名单
- GUI 设备页选择设备 → Rust 引擎启动时按 VID/PID 过滤
- 懒加载：首次收到事件时解析硬件 ID，支持热插拔
- 未白名单设备透传原始事件，正常使用

### GUI 设备页
- Rust 引擎卡片显示设备列表（和 AHI 同级）
- 设备别名按类型独立存储（键盘/鼠标各行独立键名）
- 别名编辑即时自动保存（FocusOut 触发）

## Bug 修复

- **鼠标按键失效**：Interception 鼠标 filter 从 0x0F（仅左右键）改为 0xFFFF（全部按键）
- **鼠标中键/侧键无事件**：Interception MouseStroke state 从纯 button code 改为位标志（0x001-0x200）
- **MouseMove 绝对坐标**：flags 从 1（绝对）改为 0（相对）
- **VID 解析全 0**：HWID 格式 `HID\VID_046D&...` 的 `VID_` 不在 `&` 段开头，改用 `find()` 定位
- **启动时 VID=0000**：`interception_get_hardware_id` 需先 receive 才返回有效值，改为首次事件时懒查
- **中文路径 DLL 加载失败**：`LoadLibraryA` 改为 `LoadLibraryW` + UTF-16 编码
- **设备白名单不生效**：`subscribed_devices` 字段被 `serde(rename_all="camelCase")` 错误转换为 `subscribedDevices`，加 `#[serde(rename)]` 覆盖
- **GUI 设备别名无法保存**：组合设备（同 VID/PID 的键盘+鼠标）共用键导致互相覆盖，改为 `VID_{vid}&PID_{pid}_{type}` 独立键
- **GUI 设备别名编辑被禁用**：`_update_device_checkboxes_state` 误将别名输入框 `state=disabled`
- **调试日志格式不统一**：对齐 AHK 格式（QPC 微秒时间戳、PIPE/COMBO/TD/INPUT/SEND 标签、详细 Phase 信息）

## 测试

- 新增 4 个单元测试：鼠标键名映射、鼠标按钮 TD hold、MouseMove 解析、Defer 打断 DoubleTap
- 总计 55 条：17 单元测试 + 1 场景测试（37 场景子测试）

## 发布

- `build.bat` 新增 Rust 引擎目录复制：`engines\rust\anykey-engine.exe` + `interception.dll`

---

# AnyKey v0.8.0 — 更新日志

## 新功能

### Per-Layer TapDance（多层 TD）
- 每个物理键在每个层独立拥有 tap/hold/doubleTap/doubleHold 四组行为
- 层间 TD 互不干扰，无需依赖 baseLayer 回退
- GUI 中为每个层独立编辑 TD 映射

### 键盘键面可视化标注
- 按键根据 TD 设置自动分区标注
  - 1 个值：居中大字显示
  - 2 个值：左右/上下/对角二分，文字偏角 35%
  - 3 个值：丁字分割
  - 4 个值：十字四分
- 每个分区有独立背景色（四角不同色阶）
- 缺失 tap 时自动以键名补全

### 输出值智能格式化
- `{Backspace}` → `Bs`，`{Enter}` → `Ent` 等缩写
- 方向键显示箭头符号（↑↓←→）
- `RUN:` 开头显示红色 R
- 字符串宏显示蓝色 M
- 大小写不敏感匹配

### 调试系统增强
- 调试日志状态持久化（下次启动自动恢复）
- 关闭调试时自动打开日志文件
- GUI 层键排序支持

## Bug 修复

- **AHI holding 释放不触发**：`_TapDanceUp` 中 holding 分支提前到层检查之前
- **tdState finished 状态卡死**：`_TapDanceDown` 中处理 finished → reset 状态，避免事件被吞
- **PhaseUp5Final 清理 tdState**：加入 finished 条目删除逻辑
- **Map Delete 空 key 卡死**：所有 `Map.Delete()` 增加 `Has()` 保护
- **嵌套 Map `.property` 访问卡死**：`keys[_key][_layer].hold` 等链式点语法改为括号语法
- **64 位 WPARAM/LPARAM 溢出**：`DefWindowProcW` 设置 `argtypes`，修复 `c_void_p` 转 None 问题
- **`_ForceHoldLayerKey` 缺少 holdValue**：释放时无法解析层键输出
- **`_TD_InterruptAll` 打断 holding 键**：保护 holding/double_holding 不被拦截
- **睡眠唤醒托盘重建**：监听 `TaskbarCreated` 消息自动重建

## 架构变更

- **管道 Phase5/6 职责重新划分**：Phase5 决策 TD 行为，Phase6 统一联合输出
- **`_TapDanceDown` 签名简化**：移除冗余 `logicalKey` 参数
- **Phase5 tdKey 重定向删除**：移除旧 slot 别名系统的冗余代码
- **`_ResolvePendingLayer` → `_FlushDeferredEntry`**：完成 Defer 系统重构
- **`keyState` 统一展望**：未来将 `tdState/comboState/keyDownMapping` 合并为统一 `keyState`
