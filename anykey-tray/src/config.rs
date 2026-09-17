//! 配置文件的**读取**侧 —— 只读那些"属于托盘 / 机器属性"的顶层字段。
//!
//! 为什么单独一个模块：`anykey_config.json` 是**一份**文件（单一数据源），
//! 里面既有映射配置（GUI 写）也有机器属性（托盘写）与后端选择（GUI 写，托盘读）。
//! 放在一处，是为了让"谁写、谁读"一眼看得清，也避免 `engine.rs` 为了读一个字段
//! 而反向依赖 `app.rs`。
//!
//! 规则（与设计文档 §16 一致）：**谁拥有开关，谁写这个字段**；
//! 读者只做"缺省 + 归一化"，绝不改写文件。

use std::path::Path;

use serde_json::Value;

use crate::paths;

/// 后端取值 —— 与引擎的 `--backend=` 参数一一对应。
pub const BACKEND_DRIVER: &str = "driver";
pub const BACKEND_LLHOOK: &str = "llhook";

/// 读取配置文件的顶层值；文件缺失 / 解析失败 / 键不存在都返回 `None`。
fn load_value(base_dir: &Path, key: &str) -> Option<Value> {
    std::fs::read_to_string(paths::config_path(base_dir))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get(key).cloned())
}

/// 读一个布尔开关（缺省 `false`）。
pub fn load_bool(base_dir: &Path, key: &str) -> bool {
    load_value(base_dir, key)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 归一化后端取值 —— 纯函数，便于单测。
///
/// 语义与引擎侧完全一致：**缺省 = driver**（保持既有行为）；
/// 取值非法（拼错、类型不对）时也退回 driver，而不是报错 ——
/// 一个配置字段的笔误不该让引擎起不来。
pub fn normalize_backend(raw: Option<&str>) -> &'static str {
    match raw {
        Some(BACKEND_LLHOOK) => BACKEND_LLHOOK,
        _ => BACKEND_DRIVER,
    }
}

/// 读 `backend` 字段并归一化。
///
/// **每次调用都重新读文件，不做缓存**：这个字段的写者是 GUI（只写 config、等引擎重载），
/// 若缓存在托盘启动时，就会出现"GUI 改了 → 托盘重载 → 还是旧值"。
pub fn load_backend(base_dir: &Path) -> &'static str {
    let raw = load_value(base_dir, "backend").and_then(|v| {
        v.as_str().map(|s| s.to_string())
    });
    normalize_backend(raw.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_defaults_to_driver() {
        // 字段缺失 = driver（保持既有行为）
        assert_eq!(normalize_backend(None), BACKEND_DRIVER);
        // 非法取值也退回 driver：一个笔误不该让引擎起不来
        for bad in ["", "drv", "Driver", "LLHOOK", "auto", "ll-hook"] {
            assert_eq!(normalize_backend(Some(bad)), BACKEND_DRIVER, "bad={:?}", bad);
        }
    }

    #[test]
    fn backend_accepts_llhook_exactly() {
        assert_eq!(normalize_backend(Some("llhook")), BACKEND_LLHOOK);
        assert_eq!(normalize_backend(Some("driver")), BACKEND_DRIVER);
    }

    /// 读文件这条路：文件缺失 / JSON 半截 / 字段类型不对，都必须安静地退回 driver
    /// （一个配置字段的问题不该让引擎起不来，这是 `normalize_backend` 的设计前提）。
    #[test]
    fn load_backend_reads_file_and_survives_garbage() {
        let dir = std::env::temp_dir().join("anykey-tray-config-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("anykey_config.json");

        // 1) 文件不存在 → driver
        let _ = std::fs::remove_file(&p);
        assert_eq!(load_backend(&dir), BACKEND_DRIVER, "missing file");

        // 2) 正常值
        std::fs::write(&p, r#"{"backend":"llhook"}"#).unwrap();
        assert_eq!(load_backend(&dir), BACKEND_LLHOOK, "llhook");

        std::fs::write(&p, r#"{"backend":"driver"}"#).unwrap();
        assert_eq!(load_backend(&dir), BACKEND_DRIVER, "driver");

        // 3) 类型不对（数字）→ driver
        std::fs::write(&p, r#"{"backend":123}"#).unwrap();
        assert_eq!(load_backend(&dir), BACKEND_DRIVER, "non-string");

        // 4) 半截 JSON（GUI 保存与托盘读取撞车时的真实形态）→ driver，且不 panic
        std::fs::write(&p, r#"{"backend":"llh"#).unwrap();
        assert_eq!(load_backend(&dir), BACKEND_DRIVER, "truncated json");

        // 5) 其它字段照常可用（证明没有因为 backend 的问题整体失效）
        std::fs::write(&p, r#"{"debug_enabled":true}"#).unwrap();
        assert!(load_bool(&dir, "debug_enabled"), "debug_enabled");
        assert_eq!(load_backend(&dir), BACKEND_DRIVER, "no backend key");

        let _ = std::fs::remove_file(&p);
    }
}
