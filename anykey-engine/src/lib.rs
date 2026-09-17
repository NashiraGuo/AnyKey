pub mod config;
pub mod state;
pub mod util;
pub mod pipeline;
pub mod emit;
#[cfg(feature = "filter-driver")]
pub mod backend;
// 事件结构体（AnyKeyInputEvent / AnyKeyOutputEvent …）目前仍定义在 filter_driver.rs 里，
// 所以这两个模块暂时与它同 cfg —— 这不代表它们"属于驱动"。
// Step 5 会把事件结构体搬到中性模块，届时它们就能在无 filter-driver 特性的构建里独立编译。
#[cfg(feature = "filter-driver")]
pub mod hook_input;
#[cfg(feature = "filter-driver")]
pub mod sendinput_out;

#[cfg(feature = "filter-driver")]
pub mod filter_driver;
pub mod app_sensor;
pub mod registry;
pub mod matcher;
pub mod runtime_builder;
