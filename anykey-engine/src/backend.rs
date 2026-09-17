//! 后端抽象 —— 把"与哪个 I/O 后端打交道"这件事收敛到本文件。
//!
//! 两个后端：
//! - `Driver`（内核过滤驱动）：全能力，含 per-device，需要装驱动 + testsigning。
//! - `LlHook`（免驱动，便携模式）：`WH_KEYBOARD_LL` 输入 + `SendInput` 输出，
//!   无 per-device，不提权时提权窗口内映射静默不生效（按键仍原生工作）。
//!
//! 四条规则：
//! 1. 后端在**构造期**确定、之后不可变；运行期没有 `if is_llhook` 这类判断。
//! 2. 每个动作一个薄方法，`match` **只允许出现在本文件**。
//! 3. 方法名沿用被包装类型的原名（`poll_all` / `send_output` / …），以压缩调用点改动面。
//! 4. **选择逻辑（`choose_backend`）是纯函数** —— 回退规则可以单测，不依赖机器状态。

#[cfg(feature = "filter-driver")]
use crate::filter_driver::{
    AnyKeyDeviceInfo, AnyKeyEnumDevicesRequest, AnyKeyInputEvent, AnyKeyMouseEvent,
    AnyKeyMouseOutputEvent, AnyKeyOutputEvent, FilterDriver, ANYKEY_DEV_FLAG_VIRTUAL,
    ANYKEY_FLAG_DEVICE_CHANGED,
};
#[cfg(feature = "filter-driver")]
use crate::hook_input::{HookInput, HookLiveness};

/// 用户请求的后端（= `--backend=` 的取值语义）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackendKind {
    /// 优先驱动；不可用则**回退**到 llhook（见 `choose_backend`）
    Driver,
    /// 强制 llhook，完全不尝试驱动
    LlHook,
}

impl BackendKind {
    pub fn name(self) -> &'static str {
        match self {
            BackendKind::Driver => "driver",
            BackendKind::LlHook => "llhook",
        }
    }
}

/// 解析 `--backend=` 的值。大小写不敏感；**未知取值返回 None**，
/// 由调用方决定是"告警 + 用缺省"还是"报错退出"（当前取前者，见 main.rs）。
pub fn parse_backend_kind(v: &str) -> Option<BackendKind> {
    match v.trim().to_ascii_lowercase().as_str() {
        "driver" => Some(BackendKind::Driver),
        "llhook" => Some(BackendKind::LlHook),
        _ => None,
    }
}

/// 纯函数：给定"用户请求"与"驱动此刻是否可用"，决定实际用哪个后端。
///
/// - 请求 `LlHook` → 恒 `LlHook`（显式选择，驱动可用也不用）
/// - 请求 `Driver` + 可用 → `Driver`
/// - 请求 `Driver` + 不可用 → **回退 `LlHook`**（不退出、不回报托盘）
///
/// 之所以做成纯函数：回退规则是本方案最容易写错、也最需要回归的部分，
/// 而它的输入只有这两个布尔/枚举维度。
pub fn choose_backend(pref: BackendKind, driver_available: bool) -> BackendKind {
    match pref {
        BackendKind::LlHook => BackendKind::LlHook,
        BackendKind::Driver => {
            if driver_available {
                BackendKind::Driver
            } else {
                BackendKind::LlHook
            }
        }
    }
}

pub enum Backend {
    Driver(FilterDriver),
    LlHook(HookInput),
}

impl Backend {
    /// 后端名 —— 只用于日志与状态上报
    pub fn name(&self) -> &'static str {
        match self {
            Backend::Driver(_) => "driver",
            Backend::LlHook(_) => "llhook",
        }
    }

    /// 等待输入（键盘 + 鼠标），最多 `timeout_ms` 毫秒，超时返回空元组。
    ///
    /// **超时语义是契约的一部分**：主循环用 `pipeline.next_deadline_ms()` 当超时值，
    /// 靠"返回空元组"这一刻推进 combo / tap-dance / defer 的定时器。
    pub fn poll_all(
        &self,
        timeout_ms: u32,
    ) -> Result<(Vec<AnyKeyInputEvent>, Vec<AnyKeyMouseEvent>), String> {
        match self {
            Backend::Driver(fd) => fd.poll_all(timeout_ms),
            Backend::LlHook(h) => h.poll_all(timeout_ms),
        }
    }

    /// 注入一个键盘输出事件
    pub fn send_output(&self, ev: &AnyKeyOutputEvent) -> Result<(), String> {
        match self {
            Backend::Driver(fd) => fd.send_output(ev),
            Backend::LlHook(h) => h.send_output(ev),
        }
    }

    /// 注入一个鼠标输出事件
    pub fn send_mouse_output(&self, ev: &AnyKeyMouseOutputEvent) -> Result<(), String> {
        match self {
            Backend::Driver(fd) => fd.send_mouse_output(ev),
            Backend::LlHook(_) => crate::sendinput_out::send_mouse_output(ev),
        }
    }

    /// 开关某设备（0 = 全部）的拦截态。
    /// 免驱动后端下这是**空操作** —— 它的"闸门"就是钩子装上/卸下本身（构造/析构即开关）。
    pub fn set_intercept(&self, device_id: u32, enable: bool) -> Result<(), String> {
        match self {
            Backend::Driver(fd) => fd.set_device_intercept(device_id, enable),
            Backend::LlHook(_) => Ok(()),
        }
    }

    /// 设备总数
    pub fn device_count(&self) -> Result<u32, String> {
        match self {
            Backend::Driver(fd) => fd.get_device_count(),
            // 便携模式只有一个伪设备 0（见 `enum_devices`）
            Backend::LlHook(_) => Ok(1),
        }
    }

    /// 枚举设备（供 Registry 扫描使用）。
    ///
    /// 便携模式返回**单设备桩**：`device_id = 0`、同时标记为键盘与鼠标。
    /// 标记为两者是为了让 `kb_target_dev` / `mouse_target_dev` 的类型判定都能落到它
    /// （便携模式没有设备维度，键鼠共用同一个路由目标）。
    pub fn enum_devices(
        &self,
        req: &AnyKeyEnumDevicesRequest,
        buf: &mut [AnyKeyDeviceInfo],
    ) -> Result<u32, String> {
        match self {
            Backend::Driver(fd) => fd.enum_devices(req, buf),
            Backend::LlHook(_) => {
                if req.index == 0 && !buf.is_empty() {
                    buf[0] = llhook_stub_device();
                    Ok(1)
                } else {
                    Ok(0)
                }
            }
        }
    }

    /// 是否发生了设备热插拔变更。
    /// 驱动侧读状态会顺带清掉那个一次性标志；免驱动后端恒为 `false`
    /// （⚠️ 不要在这里返回 Err —— 主循环每轮都会调它，会刷日志）。
    pub fn device_changed(&self) -> Result<bool, String> {
        match self {
            Backend::Driver(fd) => fd
                .get_status()
                .map(|st| st.flags & ANYKEY_FLAG_DEVICE_CHANGED != 0),
            Backend::LlHook(_) => Ok(false),
        }
    }

    /// 存活对账。驱动后端由心跳线程 + 驱动看门狗负责 → `None`；
    /// 免驱动后端返回钩子对账结果（系统侧有输入而钩子长时间静默 = 疑似失活）。
    pub fn liveness(&self) -> Option<HookLiveness> {
        match self {
            Backend::Driver(_) => None,
            Backend::LlHook(h) => Some(h.liveness()),
        }
    }
}

/// 便携模式的伪设备。名字里带 portable 是为了让日志一眼能看出当前后端。
#[cfg(feature = "filter-driver")]
fn llhook_stub_device() -> AnyKeyDeviceInfo {
    let mut info: AnyKeyDeviceInfo = unsafe { std::mem::zeroed() };
    info.device_id = 0;
    info.is_keyboard = 1;
    info.is_mouse = 1;
    info.flags = ANYKEY_DEV_FLAG_VIRTUAL;
    let name: Vec<u16> = "AnyKey portable (single merged device)"
        .encode_utf16()
        .collect();
    let n = name.len().min(info.friendly_name.len() - 1);
    info.friendly_name[..n].copy_from_slice(&name[..n]);
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_known_values_case_insensitively() {
        assert_eq!(parse_backend_kind("driver"), Some(BackendKind::Driver));
        assert_eq!(parse_backend_kind("DRIVER"), Some(BackendKind::Driver));
        assert_eq!(parse_backend_kind(" llhook "), Some(BackendKind::LlHook));
        assert_eq!(parse_backend_kind("LlHook"), Some(BackendKind::LlHook));
    }

    #[test]
    fn parse_rejects_unknown_and_empty() {
        // "auto" 是刻意不支持的取值（多一个取值没有对应需求）
        for v in ["auto", "", "drv", "ll", "driver2", "--backend=driver"] {
            assert_eq!(parse_backend_kind(v), None, "unexpectedly parsed: {:?}", v);
        }
    }

    #[test]
    fn llhook_request_never_uses_driver() {
        assert_eq!(choose_backend(BackendKind::LlHook, true), BackendKind::LlHook);
        assert_eq!(choose_backend(BackendKind::LlHook, false), BackendKind::LlHook);
    }

    #[test]
    fn driver_request_falls_back_only_when_unavailable() {
        assert_eq!(choose_backend(BackendKind::Driver, true), BackendKind::Driver);
        assert_eq!(choose_backend(BackendKind::Driver, false), BackendKind::LlHook);
    }
}
