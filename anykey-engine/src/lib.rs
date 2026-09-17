pub mod config;
pub mod state;
pub mod util;
pub mod pipeline;
pub mod emit;
// 两个后端共用的 I/O 词汇（事件结构体 + 共享常量 + 鼠标包展开），无 cfg。
pub mod events;
// 免驱动后端的两个模块：只依赖 events，**不再与 filter-driver 同 cfg**。
pub mod hook_input;
pub mod sendinput_out;
// 后端抽象：**只有 Driver 变体需要驱动句柄**，所以本模块随特性走；
// LlHook 变体本身不需要，但同一个 enum 不能一半有特性一半没特性。
pub mod backend;
// ⚠️ 现有约束：`registry`（设备清单来自驱动）与本 crate 的 main（`fn main` 整体挂在该特性下）
// 仍要求 `filter-driver`。所以"无驱动的便携精简版构建"还需要单独解耦这两处；
// 那不是本步的目标（见设计文档 §5.0：不让 cfg 去决定运行期用哪个后端）。

#[cfg(feature = "filter-driver")]
pub mod filter_driver;
pub mod app_sensor;
pub mod registry;
pub mod matcher;
pub mod runtime_builder;
