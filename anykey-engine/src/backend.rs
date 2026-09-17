//! 后端抽象 —— 把"与哪个 I/O 后端打交道"这件事收敛到本文件。
//!
//! 目前只有 `Driver` 一个变体（内核过滤驱动）。免驱动的 `LlHook` 变体在下一步加入，
//! 届时 **main.rs 不需要任何改动**（只在本文件里加一个 match 分支）。
//!
//! 三条规则：
//! 1. 后端在**构造期**确定、之后不可变；运行期没有 `if is_llhook` 这类判断。
//! 2. 每个动作一个薄方法，`match` **只允许出现在本文件**。
//! 3. 方法名沿用被包装类型的原名（`poll_all` / `send_output` / …），以压缩调用点改动面。

#[cfg(feature = "filter-driver")]
use crate::filter_driver::{
    AnyKeyDeviceInfo, AnyKeyEnumDevicesRequest, AnyKeyInputEvent, AnyKeyMouseEvent,
    AnyKeyMouseOutputEvent, AnyKeyOutputEvent, FilterDriver, ANYKEY_FLAG_DEVICE_CHANGED,
};

pub enum Backend {
    Driver(FilterDriver),
}

impl Backend {
    /// 后端名 —— 只用于日志与状态上报
    pub fn name(&self) -> &'static str {
        match self {
            Backend::Driver(_) => "driver",
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
        }
    }

    /// 注入一个键盘输出事件
    pub fn send_output(&self, ev: &AnyKeyOutputEvent) -> Result<(), String> {
        match self {
            Backend::Driver(fd) => fd.send_output(ev),
        }
    }

    /// 注入一个鼠标输出事件
    pub fn send_mouse_output(&self, ev: &AnyKeyMouseOutputEvent) -> Result<(), String> {
        match self {
            Backend::Driver(fd) => fd.send_mouse_output(ev),
        }
    }

    /// 开关某设备（0 = 全部）的拦截态。
    /// 免驱动后端下这是**空操作** —— 它的"闸门"就是钩子装上/卸下本身。
    pub fn set_intercept(&self, device_id: u32, enable: bool) -> Result<(), String> {
        match self {
            Backend::Driver(fd) => fd.set_device_intercept(device_id, enable),
        }
    }

    /// 设备总数
    pub fn device_count(&self) -> Result<u32, String> {
        match self {
            Backend::Driver(fd) => fd.get_device_count(),
        }
    }

    /// 枚举设备（供 Registry 扫描使用）
    pub fn enum_devices(
        &self,
        req: &AnyKeyEnumDevicesRequest,
        buf: &mut [AnyKeyDeviceInfo],
    ) -> Result<u32, String> {
        match self {
            Backend::Driver(fd) => fd.enum_devices(req, buf),
        }
    }

    /// 是否发生了设备热插拔变更。
    /// 驱动侧读状态会顺带清掉那个一次性标志；免驱动后端恒为 `false`。
    pub fn device_changed(&self) -> Result<bool, String> {
        match self {
            Backend::Driver(fd) => fd
                .get_status()
                .map(|st| st.flags & ANYKEY_FLAG_DEVICE_CHANGED != 0),
        }
    }
}
