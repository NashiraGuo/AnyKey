/// AnyKey engine - Config types
/// Matches Python GUI's anykey_config.json (camelCase + serde rename)

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

static EMPTY_KEY_MAP: LazyLock<HashMap<String, KeyEntry>> = LazyLock::new(HashMap::new);

/// Top-level config
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_combo_time")]
    pub combo_time: u64,

    #[serde(default)]
    pub combo_map: Vec<ComboRow>,

    #[serde(default, rename = "tapDance")]
    pub layers: Layers,

    #[serde(default)]
    pub leader: LeaderConfig,

    #[serde(default)]
    #[serde(rename = "subscribed_devices")]
    pub subscribed_devices: Vec<DeviceInfo>,

    /// 「设备独立设置」开关（JSON 键 perDevice）：
    ///   - false（默认，不启用独立设置）：只有全局设置生效，所有设备都经过全局映射；
    ///   - true（启用独立设置）：只有被订阅的设备走各自 map，未订阅设备透传。
    /// 构建 runtime 时据此决定订阅设备集。
    #[serde(default)]
    #[serde(rename = "perDevice")]
    pub per_device: bool,

    /// per-device 映射覆盖桶（覆盖-only，只存被改条目）。
    /// key = 设备 guid / "VID:PID:type"（无 guid 降级），与 device_registry 绑定协同。
    /// 解析时按 identity 合并全局 ∪ 设备覆盖：tapDance=physKey（base 层 KeyEntry）、
    /// combo=(key1,key2)、leader=序列；同 identity 设备覆盖全局，否则继承全局。
    /// 设备设置不向上同步全局。空桶 = 完全继承全局。
    #[serde(default)]
    #[serde(rename = "devices")]
    pub devices: HashMap<String, DeviceOverride>,

    /// 应用感知：按进程名覆盖映射。key = 进程名（如 "devenv.exe"）。
    #[serde(default)]
    pub app_aware: AppAwareConfig,
}

/// per-device 映射覆盖（覆盖-only）。
/// 只含被设备改写的条目；缺段/缺条目 → 继承全局对应部分。
/// 注：tapDance 计时(holdTerm 等)保持全局，不在此覆盖（引擎 state.rs 直接读 config.tap_dance）。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceOverride {
    /// 覆盖-only：被改写的 combo 行（按 (key1,key2) 身份合并）
    #[serde(default)]
    pub combo_map: Option<Vec<ComboRow>>,
    /// 覆盖-only：{层名: {physKey: KeyEntry}}；base 层 KeyEntry 同时驱动 tapDance 绑定
    #[serde(default, rename = "tapDance")]
    pub layers: Option<HashMap<String, HashMap<String, KeyEntry>>>,
    /// 覆盖-only：被改写的 leader 序列（按序列身份合并）
    #[serde(default)]
    pub leader: Option<Vec<LeaderSequence>>,
    /// 设备专属应用覆盖：{进程名: AppOverride}。dev1.apps 不影响 dev2。
    #[serde(default)]
    pub apps: Option<HashMap<String, AppOverride>>,
}

fn default_combo_time() -> u64 { 200 }
fn default_ht() -> u64 { 150 }
fn default_dt() -> u64 { 250 }
fn default_dh() -> u64 { 150 }

/// Device VID/PID for subscription filtering (matches GUI device tab)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    #[serde(default)]
    pub vid: String,
    #[serde(default)]
    pub pid: String,
    #[serde(default)]
    pub alias: String,
    #[serde(default)]
    pub guid: Option<String>,          // ContainerID GUID
    #[serde(default, alias = "runtime_device_id")]
    pub runtime_device_id: Option<u32>, // same-model dual keyboard lock (GUI writes snake_case "runtime_device_id")
    #[serde(default, alias = "hardware_id")]
    pub hardware_id: Option<String>,    // exact HardwareId for precise matching (GUI populates in step 4)
    #[serde(default = "default_kind", alias = "type")]
    pub kind: String,                   // "keyboard" | "mouse"
    #[serde(default, alias = "domain_id")]
    pub domain_id: Option<u32>,         // None=auto→domain 1, 0=独立 runtime, N=共享 domain N
    #[serde(default)]
    pub enabled: bool,                  // user checked in GUI
}

fn default_kind() -> String { "keyboard".into() }

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComboRow {
    pub key1: String,
    pub key2: String,
    pub output: String,
    #[serde(default = "default_layer")]
    pub layer: String,
}

fn default_layer() -> String { "base".into() }

/// 扁平 tapDance 格式。计时字段具名，层数据由 #[serde(flatten)] 捕获为 HashMap。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layers {
    #[serde(default = "default_ht")]
    pub hold_term: u64,

    #[serde(default = "default_dt")]
    pub double_tap_term: u64,

    #[serde(default = "default_dh")]
    pub double_hold_term: u64,

    /// key=层名（"base","fn1","fn2"...）, value={physKey: KeyEntry}
    #[serde(flatten)]
    pub layer_maps: HashMap<String, HashMap<String, KeyEntry>>,
}

impl Default for Layers {
    fn default() -> Self {
        Self {
            hold_term: default_ht(),
            double_tap_term: default_dt(),
            double_hold_term: default_dh(),
            layer_maps: HashMap::new(),
        }
    }
}

impl Layers {
    /// 获取 base 层
    pub fn base(&self) -> &HashMap<String, KeyEntry> {
        // 立即求值常量 HashMap 无法在 const 中创建，用 lazy / once_cell 替代
        // 直接 field access 更快
        &self.layer_maps
            .get("base")
            .or_else(|| self.layer_maps.get("baseLayer"))
            .unwrap_or(&EMPTY_KEY_MAP)
    }

    /// 迭代所有 fn1, fn2... 层名（排序）
    pub fn fn_layers(&self) -> Vec<&String> {
        let mut names: Vec<&String> = self.layer_maps
            .keys()
            .filter(|k| k.starts_with("fn"))
            .collect();
        names.sort_by_key(|k| k[2..].parse::<u32>().unwrap_or(0));
        names
    }

    /// 按名称取层
    pub fn get(&self, name: &str) -> Option<&HashMap<String, KeyEntry>> {
        self.layer_maps.get(name)
    }
}

/// Per-key config from GUI (tap/hold/ht/dt/dh/dtt/dht)
#[derive(Debug, Clone, Deserialize, Default)]
pub struct KeyEntry {
    #[serde(default)]
    pub tap: String,
    #[serde(default)]
    pub hold: String,
    #[serde(default)]
    pub ht: String,
    #[serde(default)]
    pub dt: String,
    #[serde(default)]
    pub dtt: String,
    #[serde(default)]
    pub dh: String,
    #[serde(default)]
    pub dht: String,
}
/// - `trigger` 配置即 blocking（拦截态），不配置(空)即 non-blocking（常驻监听）
/// - `sequences[i].keys.len() >= 2`，否则启动时跳过并 warning
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaderConfig {
    /// 滑动窗口超时（毫秒）
    #[serde(default = "default_leader_timeout")]
    pub timeout_ms: u64,

    #[serde(default)]
    pub sequences: Vec<LeaderSequence>,

    /// 循环捕获：leader 输出键回灌进匹配，驱动序列自动串联（如 12→3→123→4→…→bingo）。
    /// false（默认）= 维持原版，输出不进栈；true = 放行 __leader_ 合成键入栈并级联触发。
    #[serde(default)]
    pub loop_capture: bool,
}

fn default_leader_timeout() -> u64 { 2000 }

impl Default for LeaderConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_leader_timeout(),
            sequences: vec![],
            loop_capture: false,
        }
    }
}

/// 单条 leader 序列：依次输入 `keys` 后执行 `output`
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LeaderSequence {
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub output: String,
    /// 本条序列专属超时（毫秒）。0 = 用公共 `LeaderConfig.timeoutMs`。
    /// 引擎在滑动窗口中，取「当前栈是前缀的所有候选序列」里 timeout_ms 最大的那条。
    #[serde(default)]
    pub timeout_ms: u64,
}

/// 应用感知配置段。key = 进程名，value = 该应用的覆盖项。
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct AppAwareConfig {
    #[serde(default)]
    pub apps: HashMap<String, AppOverride>,
}

/// 应用的映射覆盖。所有字段均为 Option，None = 继承设备默认。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppOverride {
    #[serde(default)]
    pub combo_time: Option<u64>,
    #[serde(default)]
    pub combo_map:  Option<Vec<ComboRow>>,
    #[serde(default)]
    pub leader:     Option<Vec<crate::config::LeaderSequence>>,
    #[serde(default, rename = "tapDance")]
    pub layers:     Option<HashMap<String, HashMap<String, KeyEntry>>>,
}
