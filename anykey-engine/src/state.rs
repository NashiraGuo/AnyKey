/// AnyKey 引擎 — 管道状态定义
/// 对应 AHI 脚本中的全局状态变量

use std::collections::{HashMap, HashSet, BinaryHeap};
use std::cmp::Reverse;
use std::time::Instant;
use std::sync::{OnceLock, Arc};
use crate::config::{Config, LeaderSequence};

/// ForceHold 触发键哨兵：combo 命中时 phase3_combo 写入 ctx.force_hold_down，
/// 由 P6 ForceHold 的 force_hold_execute（defer.rs）识别后全激活整栈 waiting 键。
pub const FORCE_HOLD_COMBO_ALL: &str = "__FORCE_HOLD_COMBO_ALL__";

/// 引擎启动时刻（用于 tick_ms 与 qpc_us 统一的时间基线）
fn engine_start() -> std::time::Instant {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

fn now_str() -> String {
    // 墙体时间 HH:mm:ss
    use std::time::SystemTime;
    let wall = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(String::new(), |d| {
        let ms = d.as_millis();
        let h = ((ms / 3600000 + 8) % 24) as u64; // UTC+8
        let m = ((ms / 60000) % 60) as u64;
        let s = ((ms / 1000) % 60) as u64;
        format!("{:02}:{:02}:{:02}", h, m, s)
    });
    // tick ms（引擎运行时间，对应 AHK A_TickCount % 1000）
    let tick_ms = engine_start().elapsed().as_millis();
    // QPC 微秒（对应 AHK QueryPerformanceCounter）
    let qpc_us = get_qpc_us();
    format!("{}.{:03}.{:03}", wall, tick_ms as u64 % 1000, qpc_us % 1000)
}

fn get_qpc_us() -> u64 {
    engine_start().elapsed().as_micros() as u64
}

// ═══════════════════════════════════════════
// Phase Flag 常量（对应 AHI 的 F_INTERRUPT 等）
// ═══════════════════════════════════════════

pub const F_INTERRUPT: u32 = 1;
pub const F_CLEANUP:    u32 = 2;
pub const F_REPEAT:     u32 = 4;
pub const F_COMBO:      u32 = 8;
pub const F_DEFER:      u32 = 16;   // P4: 合并原 phase4_flush(发送 defer 键) + phase5_defer(延迟决策)；不主动 force-hold
pub const F_TAPDANCE:   u32 = 32;   // P5: tap dance 状态机
pub const F_FORCEHOLD:  u32 = 64;   // P6: 独立 ForceHold 相位（置于 commit_stage 前）；消费 ctx.force_hold_down 激活 waiting switch 键
pub const F_COMMIT:    u32 = 128;  // P7: commit_stage（resolver + output，合并了原 P7 resolve + P8 final）

// ── KeyUp 阶段 flags ─────────────────────────────────────────────
// 与 keydown 阶段 flag 完全独立、位域不重叠（≥1024，预留未来 reenter_up 合并空间）。
// keyup 为顺序直调（key_up），各 phase 顶部用 `if ctx.flags & F_UPx != 0 { return; }` 门控；
// 阶段连续编号（与 keydown 侧相位对称）：
//   up1 combo → up2 flush → up3 td → up4 forcehold → up5 commit(含 resolve+output)
//   → up6 release(phase_up6_release: release_key) → up7 flushbehind → up8 clean
// 默认 key_up 置 `F_UPCOMMIT`（跳过 up5_commit），即「纯释放」路径；
// tap/doubletap/延迟键等需「现发新输出」的分支清掉这个 skip 标志进入；up4_forcehold 默认不跑（subset 为空，no-op）。
pub const F_UPCOMBO:        u32 = 1024;  // phase_up1_combo
pub const F_UPFLUSH:        u32 = 2048;  // phase_up2_flush
pub const F_UPTAPDANCE:     u32 = 4096;  // phase_up3_td（tap_dance_up）
pub const F_UPFORCEHOLD:    u32 = 8192;  // phase_up4_forcehold
pub const F_UPCOMMIT:      u32 = 16384; // phase_up5_commit（resolver + output）

pub const F_UPRELEASE:      u32 = 32768;  // phase_up6_release（release_key）
pub const F_UPFLUSHBEHIND:  u32 = 65536;  // phase_up7_flushbehind
pub const F_UPCLEAN:        u32 = 131072; // phase_up8_clean

// ── 阶段跳过集（语义化命名）────────────────────────────────────
// invariant：以上 F_* 必须是「从 1 开始的连续 2 的幂」（1,2,4,...,128）。
// 新增阶段时追加下一个 2 的幂（如 256, 512）并更新 ALL_PHASE_FLAGS 即可，
// 下方所有跳过集通过 skip_from / skip_before 自动包含新阶段，调用点无需改动。
pub const ALL_PHASE_FLAGS: u32 =
    F_INTERRUPT | F_CLEANUP | F_REPEAT | F_COMBO | F_DEFER
  | F_TAPDANCE | F_FORCEHOLD | F_COMMIT;

/// 跳过从 `from` 起（含）往后的所有阶段 —— 用于「本次按键静默，等定时器/re_enter 接手」
const fn skip_from(from: u32) -> u32 { ALL_PHASE_FLAGS & !(from - 1) }
/// 跳过 `first_run` 之前（不含）的所有阶段 —— 用于 re_enter 从某阶段重启
const fn skip_before(first_run: u32) -> u32 { first_run - 1 }

// re_enter 重播跳过集（调用点语义自解释）
pub const COMBO_TIMEOUT_REENTER: u32 = skip_before(F_DEFER);        // 跳过 P0~P3，从 P4 重启
pub const FLUSH_DEFERRED_REENTER: u32 = skip_before(F_FORCEHOLD);   // 跳过 P0~P5，经 P6→P7(commit_stage)
pub const COMMIT_SEND_REENTER:   u32 = skip_before(F_COMMIT);     // 跳过 P0~P6，P7(commit_stage) 含 resolve+output

// 阶段内静默集：本次按键到此打住，等定时器/re_enter 接手
pub const SILENCE:           u32 = skip_from(F_DEFER);              // 跳过 P4~P7（defer/TD/ForceHold/commit_stage）；P0~P3 已跑
pub const COMBO_REPEAT_SKIP: u32 = skip_from(F_TAPDANCE) & !F_COMMIT;  // 跳过 P5~P6，P7(commit_stage) 正常跑

// ── KeyUp 跳过集 ─────────────────────────────────────────────
pub const REENTER_UP_SKIP: u32 = F_UPCOMBO | F_UPFLUSH | F_UPTAPDANCE | F_UPFLUSHBEHIND;

/// TD 动作（替换原来的字符串魔法值）
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TdAction {
    #[default]
    None,
    Tap,
    Hold,
    Holding,
    Intercept,
    DoubleTap,
    HoldRelease,
    DoubleHoldRelease,
    TwoTap,
    /// doublehold 到期确认输出
    DoubleHold,
}

impl TdAction {
    /// 转换为 resolve_key_output 使用的动作名
    pub fn as_resolve_key(&self) -> &str {
        match self {
            TdAction::Tap => "tap",
            TdAction::Hold => "hold",
            TdAction::Holding => "hold",
            TdAction::Intercept => "",
            TdAction::DoubleTap => "doubleTap",
            TdAction::HoldRelease => "hold",
            TdAction::DoubleHoldRelease => "doubleHold",
            TdAction::TwoTap => "tap",
            TdAction::None => "",
            TdAction::DoubleHold => "doubleHold",
        }
    }
}

/// 上下文（对应 AHK 的 ctx 对象）
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub key: String,
    pub logical_key: String,
    pub flags: u32,
    pub td_action: TdAction,
    pub td_pending: bool,
    pub clear_combo: bool,
    pub td_force: bool,
    /// ForceHold 触发键（keydown 专用）：分支(phase_defer combo 命中 / flush_deferred_entry)写入本键，
    /// 交 phase_forcehold(P6) 执行。None=本 keydown 无 force-hold 需求。
    /// - combo 命中写哨兵 FORCE_HOLD_COMBO_ALL → 函数全激活整栈；
    /// - 普通键写键名 → P6 激活其【之前】(不含自己)的 waiting switch 键，再移除自己。
    pub force_hold_down: Option<String>,
    /// ForceHold 触发键（keyup 专用）：分支(phase_up2_flush / phase_up1_combo Seeking / tap_dance_up Waiting)写入本键，
    /// 交 phase_up4_forcehold(up4) 执行。语义同 force_hold_down，但字段独立避免跨侧串扰（同 Context 复用不互污）。
    pub force_hold_up: Option<String>,
}

impl Context {
    pub fn new(key: &str) -> Self {
        let nk = crate::util::norm_key(key);
        Context {
            key: nk.clone(),
            logical_key: format!("{{{}}}", nk),
            ..Default::default()
        }
    }
}


#[derive(Debug, Clone, Default)]
pub struct KeysState {
    pub key_states:    HashMap<String, KeyState>,
    pub pending_timers: HashMap<String, TimerId>,
}
#[derive(Debug, Clone, Default)]
pub struct DeferState {
    pub waiting_stack:  Vec<(String, SwitchKind)>,
    pub keys_deferred:  HashMap<String, DeferInfo>,
}
#[derive(Debug, Clone, Default)]
pub struct RuntimeLayerState {
    pub layer_stack:  Vec<LayerEntry>,
}
#[derive(Debug, Clone, Default)]
pub struct RuntimeLeaderState {
    pub leader_on:    bool,
    pub leader_stack: Vec<(String, String)>,
    pub leader_pending: Option<String>,
    pub leader_pending_phys: Option<String>,
}

// ── DeviceContext (per-device mapping + runtime) ──

/// 共享运行时域。多个 DeviceContext 可通过 domain_id 引用同一份 runtime。
/// 设备负责映射（tap_dance/combo/leader），RuntimeDomain 负责运行时状态（layer/combo state/td state）。
#[derive(Debug, Clone)]
pub struct RuntimeDomain {
    pub domain_id: u32,
    pub state:     DeviceState,
}

impl RuntimeDomain {
    pub fn new(domain_id: u32) -> Self {
        Self {
            domain_id,
            state: DeviceState {
                keys:   KeysState::default(),
                defer:  DeferState::default(),
                layers: RuntimeLayerState::default(),
                leader: RuntimeLeaderState::default(),
            },
        }
    }
}

/// 每个设备的上下文：映射数据独立，运行时通过 domain_id 共享。
#[derive(Debug, Clone)]
pub struct DeviceContext {
    pub mapping:   DeviceMapping,
    pub domain_id: u32,  // 指向 RuntimeDomain
}

/// 一个设备的映射数据
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceMapping {
    pub tap_dance: HashMap<String, HashMap<String, KeyDef>>,
    pub combo:     ComboIndex,
    pub leader:    LeaderDef,
}

/// 一个设备的运行时状态
#[derive(Debug, Clone, Default)]
pub struct DeviceState {
    pub keys:   KeysState,
    pub defer:  DeferState,
    pub layers: RuntimeLayerState,
    pub leader: RuntimeLeaderState,
}

/// Combo 映射数据——从 config 一次性构建
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComboIndex {
    pub map:      HashMap<String, HashMap<String, HashMap<String, String>>>,  // 层→键1→键2→输出
    pub key_set:  HashSet<String>,                                             // 所有参与 combo 的键
    pub partners: HashMap<String, HashSet<String>>,                            // 键→可组合的伙伴键集合
}

/// Leader 映射数据（从 config 读自有，once-built）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderDef {
    pub sequences:  Vec<LeaderSequence>,
    pub timeout_ms: u64,
}

/// Combo 状态（对应 AHK 的 comboState[key]）
#[derive(Debug, Clone)]
pub struct ComboState {
    pub state: ComboKind,
    pub down_time: u64,
    pub partner: String,
    pub output: String,
    /// 匹配时命中的层（fn1/bn2/base…）；供 phase7_resolve 层感知查表，
    /// 避免「匹配后激活修饰键污染 current_layer」导致层 combo 找不到输出。
    pub layer: String,
    /// Up 路径专用：本键是否为 combo 中「最后抬起」、负责发 Up 的键。
    /// 在 phase_up2_flush（resolve_key_up）置位，实际发送推迟到 phase_up7_release 的 release_key
    /// （分层：combo 阶段只写状态，发送放 final）。发送后清零。
    pub pending_up: bool,
}

impl ComboState {
    pub fn waiting(_key: &str, now: u64) -> Self {
        Self {
            state: ComboKind::Seeking,
            down_time: now,
            partner: String::new(),
            output: String::new(),
            layer: String::new(),
            pending_up: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComboKind {
    Seeking,    // 等待伙伴
    Matched,    // combo 已匹配, 待 phase_resolve 映射(输出留空, 由 P7 填充)
    Holding,    // hold combo 已触发
    Active,     // macro combo 已触发
    Stuck,      // 超时无伙伴,按键还按着
    Failed,     // combo 未完成被提前抬起,标记待 Phase1 cleanup_released 回收
    Interrupted,// 被第三键打断（combo_interrupt_all 经 re_enter 重跑后交 tapdance 解析）
    Released,   // 已释放
}

/// TapDance 状态（对应 AHK 的 tdState[key]），作为 KeyState 的 td 子系统
/// 注意：hold 解析值不再存这里——改由 phase7_final 统一记录到 KeyState.emitted（完整 {X} 形式），
/// 反激活/释放时直接读 emitted，避免重复存储导致的不一致。
#[derive(Debug, Clone)]
pub struct TdState {
    pub kind: TdKind,
}

/// 每键的持久状态容器：统一收拢 td / combo / emitted 的目标结构
/// （对应 AHK 的 tdState / comboState / keyDownMapping 三张表合一）
/// 阶段1：收拢 td + combo 子系统；emitted（key_down_mapping）在阶段2 并入
#[derive(Debug, Clone, Default)]
pub struct KeyState {
    pub td: Option<TdState>,
    pub combo: Option<ComboState>,
    pub emitted: Option<String>,
    /// 层键专属：记录本物理键当前激活的目标层（fn1/bn2…）。
    /// 与 emitted 解耦——层键不写入 emitted（emitted 留空），
    /// 反激活由本字段驱动，避免 auto-repeat 把 emitted 覆盖成物理键导致层不反激活。
    pub target_layer: Option<String>,

    /// Leader Key 拦截标记：本键被 leader 拦截（Down/Up 都被吞）。
    /// 由 leader 子系统自行清理（clear_leader_blk），不挂到 clear_td/clear_combo/clear_emitted/clear_target_layer 的删除条件。
    pub leader_blk: bool,

    /// 物理键的输入来源设备 ID（key_down 入口写入，全管道只读）。
    /// emit 时从本字段取，不再依赖 self.current_device（定时器路径无法保证其正确性）。
    pub device_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TdKind {
    Waiting,      // hold timer 等待中
    TapDone,      // tap 完成，等双击
    Holding,      // hold 激活中
    DoubleWait,   // 双击等待
    DoubleHolding,// 双击 hold 激活中
    DoubleTapHold,// 双击（无 doublehold）以「按住」方式发出，待物理键释放
    Finished,     // TD 结束
    Reset,        // finished 后的 auto-repeat → 标记重置（同 AHK "reset"）
}

/// Defer 信息
#[derive(Debug, Clone)]
pub struct DeferInfo {
    pub td_key: String,
}

/// 键定义（对应 AHK keys[phys][layer] 结构）
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyDef {
    pub tap: String,
    pub hold: String,
    pub double_tap: String,
    pub double_hold: String,
    pub hold_term: Option<u64>,
    pub dbl_tap_term: Option<u64>,
    pub dbl_hold_term: Option<u64>,
}

impl KeyDef {
    pub fn has_td(&self) -> bool {
        !self.hold.is_empty() || !self.double_tap.is_empty() || !self.double_hold.is_empty()
    }
}

/// 输出记录（用于测试断言）
#[derive(Debug, Clone, PartialEq)]
pub enum EmitEvent {
    Down(String, u32),
    Up(String, u32),
    Tap(String, u32),
    MouseDown(String, u32),  // 鼠标按下（name=鼠标键名，pipeline已判断）
    MouseUp(String, u32),    // 鼠标释放
    Text(String, u32),
    Run(String, u32),
    MouseMove(i32, i32, u32),
    TapSI(String, u32),
    DownSI(String, u32),
    UpSI(String, u32),
    LayerOn(String, u32),
    LayerOff(String, u32),
}

// ═══════════════════════════════════════════
// 定时器类型
// ═══════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TimerKind {
    Hold,
    DoubleTap,
    DoubleHold,
    ComboTimeout,
    SleepTimer,
    LeaderTimeout,
}

/// waiting_stack 入栈标签：记录一个 switch 键是因哪个字段（hold / double_hold）进入栈的。
/// 发送（force-hold / 抬起结算）时按标签走对应字段输出，避免扩大 is_switch_key 查询范围
/// （waiting_stack 既要记录又要查询发什么，见 defer 设计文档 C5.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchKind {
    /// hold=层/修饰键 → 入栈即 Waiting，force-hold 时发 hold
    Hold,
    /// double_hold=层/修饰键（doublehold-switch）→ 第二次按下确认 DoubleWait 时入栈，
    ///   force-hold / 抬起结算时发 double_hold（被强制按住）或 double_tap（快速抬起）
    DoubleHold,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TimerEntry {
    /// 唯一 id：用于「延迟删除」精确取消——取消时记录具体 id，
    /// 新调度条目拿全新 id，天然不受旧取消标记影响（修复「新建计时器复活旧计时器」bug）。
    pub id: TimerId,
    pub deadline_tick: u64,
    pub kind: TimerKind,
    pub key: String,
    pub td_key: String,
    pub extra_data: String,
    /// 调度该定时器的设备 ID。
    pub device_id: u32,
    /// 调度时的映射快照（Arc 共享）。触发时临时 swap 后用此快照处理。
    pub mapping: std::sync::Arc<DeviceMapping>,
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deadline_tick.cmp(&other.deadline_tick)
    }
}
impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// 层栈条目。记录每层激活时的完整上下文。
#[derive(Debug, Clone)]
pub struct LayerEntry {
    /// 归一化层名（fn1/fn2…），用于查 keys 表和反激活
    pub name: String,
    /// 触发该层激活的物理键
    pub activated_by: String,
    /// 解析的字段（"hold"/"doubleHold"/"tap"/"doubleTap"/"combo"）
    pub action: String,
    /// 解析出的原始输出值（"{bn1}" / "{fn2}" / "{ctrl}"）
    pub resolved_output: String,
}

// ═══════════════════════════════════════════
// 管道状态（单一体，对应 AHK 所有 global 变量）
// ═══════════════════════════════════════════

pub struct PipelineState {
    pub config: Config,
    pub tick: u64,  // 模拟时间，测试时可快进

    // v3: 当前运行时上下文。仅在 domain/app 切换时变，非每事件。
    pub mapping:        std::sync::Arc<DeviceMapping>,  // 映射快照（Arc 共享到 TimerEntry）
    pub state:          DeviceState,                    // 当前 domain 的运行时状态
    pub current_domain: u32,                            // 当前 domain
    pub current_device: u32,                            // 输出目标设备（EmitEvent stamp）
    pub current_app:    String,                         // 当前焦点应用进程名

    // 输出日志（测试用）
    pub emit_log: Vec<EmitEvent>,

    // Debug 日志
    pub debug_enabled: bool,
    pub debug_log: Vec<String>,

    // 函数调用映射（FUNC_MAP）
    pub func_map: Vec<String>,

    // 定时器 ID 计数
    timer_counter: u64,

    // 定时器优先级队列
    pub timer_heap: BinaryHeap<Reverse<TimerEntry>>,
    /// 延迟删除：被取消的定时器 id 集合 → 弹出时跳过（按 id 精确取消，而非按 (key, kind) 粗粒度）
    pub cancelled_timer_ids: HashSet<TimerId>,
}

pub type TimerId = u64;

impl PipelineState {
    // ---- KeyState 辅助访问 ----
    pub fn td(&self, key: &str) -> Option<&TdState> {
        self.state.keys.key_states.get(key).and_then(|s| s.td.as_ref())
    }
    pub fn td_mut(&mut self, key: &str) -> Option<&mut TdState> {
        self.state.keys.key_states.get_mut(key).and_then(|s| s.td.as_mut())
    }
    pub fn has_td(&self, key: &str) -> bool {
        self.state.keys.key_states.get(key).map_or(false, |s| s.td.is_some())
    }
    pub fn combo(&self, key: &str) -> Option<&ComboState> {
        self.state.keys.key_states.get(key).and_then(|s| s.combo.as_ref())
    }
    pub fn combo_mut(&mut self, key: &str) -> Option<&mut ComboState> {
        self.state.keys.key_states.get_mut(key).and_then(|s| s.combo.as_mut())
    }
    pub fn has_combo(&self, key: &str) -> bool {
        self.state.keys.key_states.get(key).map_or(false, |s| s.combo.is_some())
    }
    // 清 td/combo 后，若整条 KeyState 无其他子系统则删除
    pub fn clear_td(&mut self, key: &str) {
        if let Some(s) = self.state.keys.key_states.get_mut(key) {
            s.td = None;
            if s.td.is_none() && s.combo.is_none() && s.emitted.is_none() && s.target_layer.is_none() {
                self.state.keys.key_states.remove(key);
            }
        }
    }
    pub fn clear_combo(&mut self, key: &str) {
        if let Some(s) = self.state.keys.key_states.get_mut(key) {
            s.combo = None;
            if s.td.is_none() && s.combo.is_none() && s.emitted.is_none() && s.target_layer.is_none() {
                self.state.keys.key_states.remove(key);
            }
        }
    }
    pub fn emitted(&self, key: &str) -> Option<&String> {
        self.state.keys.key_states.get(key).and_then(|s| s.emitted.as_ref())
    }
    pub fn set_emitted(&mut self, key: &str, val: String) {
        self.state.keys.key_states.entry(key.to_string()).or_default().emitted = Some(val);
    }
    pub fn clear_emitted(&mut self, key: &str) {
        if let Some(s) = self.state.keys.key_states.get_mut(key) {
            s.emitted = None;
            if s.td.is_none() && s.combo.is_none() && s.emitted.is_none() && s.target_layer.is_none() {
                self.state.keys.key_states.remove(key);
            }
        }
    }
    pub fn target_layer(&self, key: &str) -> Option<&String> {
        self.state.keys.key_states.get(key).and_then(|s| s.target_layer.as_ref())
    }
    pub fn set_target_layer(&mut self, key: &str, val: String) {
        self.state.keys.key_states.entry(key.to_string()).or_default().target_layer = Some(val);
    }
    pub fn clear_target_layer(&mut self, key: &str) {
        if let Some(s) = self.state.keys.key_states.get_mut(key) {
            s.target_layer = None;
            if s.td.is_none() && s.combo.is_none() && s.emitted.is_none() && s.target_layer.is_none() {
                self.state.keys.key_states.remove(key);
            }
        }
    }

    // ---- Leader Key 拦截标记（Step2）----
    pub fn leader_blk(&self, key: &str) -> bool {
        self.state.keys.key_states.get(key).map_or(false, |s| s.leader_blk)
    }
    pub fn set_leader_blk(&mut self, key: &str) {
        self.state.keys.key_states.entry(key.to_string()).or_default().leader_blk = true;
    }
    pub fn clear_leader_blk(&mut self, key: &str) {
        if let Some(s) = self.state.keys.key_states.get_mut(key) {
            s.leader_blk = false;
            if s.td.is_none() && s.combo.is_none() && s.emitted.is_none() && s.target_layer.is_none() {
                self.state.keys.key_states.remove(key);
            }
        }
    }


    pub fn new(mut config: Config) -> Self {
        // Leader 配置校验（Step1）：每条序列 keys >= 2，否则跳过并 warning
        // （leader 系统始终开启，触发键固定为 {leader}，无需 enabled/trigger 配置）
        let mut leader_warnings: Vec<String> = Vec::new();
        {
            let mut kept = Vec::with_capacity(config.leader.sequences.len());
            for (i, seq) in config.leader.sequences.drain(..).enumerate() {
                if seq.keys.len() >= 2 {
                    kept.push(seq);
                } else {
                    leader_warnings.push(format!(
                        "[leader] 跳过第 {} 条序列（keys 长度 {} < 2，已忽略）",
                        i, seq.keys.len()
                    ));
                }
            }
            config.leader.sequences = kept;
        }

     // v3: 从 config 构建初始 mapping（测试兼容）
    let init_mapping = crate::runtime_builder::build_tap_dance_map(&config.layers, &config.combo_map);
    let init_combo = crate::runtime_builder::build_combo_index(&config.combo_map);
    let init_leader = LeaderDef {
        sequences: config.leader.sequences.clone(),
        timeout_ms: config.leader.timeout_ms,
    };
    let mut s = Self {
        tick: 0,
        mapping: Arc::new(DeviceMapping {
            tap_dance: init_mapping,
            combo: init_combo,
            leader: init_leader,
        }),
        state: DeviceState::default(),
        current_device: 1,
        current_domain: 1,
        current_app: String::new(),
        emit_log: vec![],
        debug_enabled: false,
        debug_log: vec![],
        func_map: vec![],
        timer_counter: 0,
        timer_heap: BinaryHeap::new(),
        cancelled_timer_ids: HashSet::new(),

        config,
    };

        for w in leader_warnings {
            s.debug_log.push(w);
        }

        s
    }

    /// 推进时间（测试用），同时触发到期的 timer 回调
    pub fn advance_ms(&mut self, ms: u64) -> u64 {
        let target = self.tick + ms;
        while self.tick < target {
            let expired = self.pop_expired_timers();
            for entry in expired {
                self.current_device = entry.device_id;
                match entry.kind {
                    TimerKind::Hold => self.hold_timer(&entry.key),
                    TimerKind::DoubleTap => self.dt_timer(&entry.key),
                    TimerKind::DoubleHold => self.dh_timer(&entry.key),
                    TimerKind::ComboTimeout => self.single_key_timeout(&entry.key),
                    TimerKind::SleepTimer => self.fire_sleep_timer(&entry),
                    TimerKind::LeaderTimeout => self.leader_timeout(),
                }
            }
            self.tick += 1;
        }
        self.tick
    }

    /// 同步 tick 到真实时间（仅更新时钟，不触发定时器）
    pub fn sync_tick_to_real_time(&mut self) {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let start = START.get_or_init(Instant::now);
        let elapsed_ms = start.elapsed().as_millis() as u64;
        if elapsed_ms > self.tick {
            self.tick = elapsed_ms;
        }
    }

    /// 当前时间戳
    pub fn now(&self) -> u64 {
        self.tick
    }

    /// 条件 debug 日志（与 AHK Debug 对应）
    /// 标签: PIPE COMBO TD SEND TIMER UP LAYER DEFER EMIT STATE LEADER
    pub fn debug(&mut self, tag: &str, msg: &str) {
        if self.debug_enabled {
            let now = now_str();
            self.debug_log.push(format!("[{}] [{:<6}] {}", now, tag, msg));
        }
    }

    /// 分配定时器 ID
    pub fn next_timer_id(&mut self) -> TimerId {
        let id = self.timer_counter;
        self.timer_counter += 1;
        id
    }

    /// 调度一个定时器（使用模拟时钟 tick）。
    /// 每个条目分配唯一 id；新条目天然不受既往取消标记影响（取消按 id 精确隔离）。
    pub fn schedule_timer(&mut self, kind: TimerKind, key: &str, td_key: &str, delay_ms: u64) {
        let deadline = self.tick + delay_ms;
        let id = self.next_timer_id();
        self.debug("TIMER", &format!("Schedule {:?} key={} delay={} deadline_tick={} id={}", kind, key, delay_ms, deadline, id));
        self.timer_heap.push(Reverse(TimerEntry {
            id,
            deadline_tick: deadline,
            kind,
            key: key.to_string(),
            td_key: td_key.to_string(),
            extra_data: String::new(),
            device_id: self.current_device,
            mapping: self.mapping.clone(),
        }));
    }

    /// 取消指定 key 的所有定时器：把堆中该 key 当前所有条目的 id 标记为取消
    pub fn cancel_timers_for_key(&mut self, key: &str) {
        self.debug("TIMER", &format!("CancelAll key={}", key));
        for Reverse(entry) in &self.timer_heap {
            if entry.key == key {
                self.cancelled_timer_ids.insert(entry.id);
            }
        }
    }

    /// 取消指定 key + kind 的定时器：把堆中当前匹配 (key, kind) 的所有条目 id 标记为取消
    pub fn cancel_timer(&mut self, key: &str, kind: TimerKind) {
        self.debug("TIMER", &format!("Cancel {:?} key={}", kind, key));
        for Reverse(entry) in &self.timer_heap {
            if entry.key == key && entry.kind == kind {
                self.cancelled_timer_ids.insert(entry.id);
            }
        }
    }

    /// 测试/调试辅助：是否存在尚未取消的 (key, kind) 定时器
    pub fn has_active_timer(&self, key: &str, kind: TimerKind) -> bool {
        self.timer_heap.iter().any(|Reverse(e)| {
            e.key == key && e.kind == kind && !self.cancelled_timer_ids.contains(&e.id)
        })
    }

    /// 获取下一个定时器的截止时间（毫秒）
    pub fn next_deadline_ms(&self) -> Option<u32> {
        self.timer_heap.peek().map(|rev| {
            let now = self.tick;
            let d = rev.0.deadline_tick;
            if d <= now { 0 } else { (d - now) as u32 }
        })
    }

    /// 弹出并返回所有到期的定时器。已取消（按 id）的条目被静默丢弃。
    pub fn pop_expired_timers(&mut self) -> Vec<TimerEntry> {
        let now = self.tick;
        let mut expired = vec![];
        loop {
            match self.timer_heap.peek() {
                Some(rev) if rev.0.deadline_tick <= now => {
                    let Reverse(entry) = self.timer_heap.pop().unwrap();
                    if self.cancelled_timer_ids.remove(&entry.id) {
                        self.debug("TIMER", &format!("Skip {:?} key={} id={} (cancelled)", entry.kind, entry.key, entry.id));
                        continue;  // 已取消 → 丢弃，继续检查下一个
                    }
                    self.debug("TIMER", &format!("Fire {:?} key={} id={}", entry.kind, entry.key, entry.id));
                    expired.push(entry);
                }
                _ => break,
            }
        }
        expired
    }

    /// 逐键 hold term（优先 per-key 配置）
    pub fn key_hold_term(&self, phys: &str, layer: &str) -> u64 {
        if let Some(layer_map) = self.mapping.tap_dance.get(phys) {
            if let Some(def) = layer_map.get(layer) {
                if let Some(ht) = def.hold_term {
                    return ht;
                }
            }
        }
        self.config.layers.hold_term
    }

    /// 逐键 double_tap_term（优先 per-key 配置）
    pub fn key_dbl_tap_term(&self, phys: &str, layer: &str) -> u64 {
        if let Some(layer_map) = self.mapping.tap_dance.get(phys) {
            if let Some(def) = layer_map.get(layer) {
                if let Some(dt) = def.dbl_tap_term {
                    return dt;
                }
            }
        }
        self.config.layers.double_tap_term
    }

    /// 逐键 double_hold_term（优先 per-key 配置）
    pub fn key_dbl_hold_term(&self, phys: &str, layer: &str) -> u64 {
        if let Some(layer_map) = self.mapping.tap_dance.get(phys) {
            if let Some(def) = layer_map.get(layer) {
                if let Some(dht) = def.dbl_hold_term {
                    return dht;
                }
            }
        }
        self.config.layers.double_hold_term
    }

    // ── Config init ──
}
