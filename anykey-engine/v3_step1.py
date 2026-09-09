# Runtime v3 refactor — step 1: state.rs struct changes

with open('src/state.rs', 'r', encoding='utf-8') as f:
    c = f.read()

# 1. Add Arc import
c = c.replace('use std::sync::OnceLock;', 'use std::sync::{OnceLock, Arc};')

# 2. Remove Runtime struct
c = c.replace("""/// 单环境（一份 mapping + 一份按键状态）。每个 (device, app) 有独立 Runtime。
#[derive(Debug, Clone)]
pub struct Runtime {
    pub mapping: DeviceMapping,
    pub state:   DeviceState,
}

""", '')

# 3. Replace PipelineState struct
old_ps = """pub struct PipelineState {
    pub config: Config,
    pub tick: u64,  // 模拟时间，测试时可快进

    // 单环境（不��� HashMap）。main 侧 resolve 后写入，pipeline 内直接字段访问。
    pub runtime: Runtime,
    pub current_device: u32,              // 当前输出目标设备（EmitEvent 盖章用）
    pub current_key: (u32, String),       // 当前环境标识（debug/log 用）

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
}"""

new_ps = """pub struct PipelineState {
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
}"""
c = c.replace(old_ps, new_ps)

# 4. Replace all self.runtime.state.xxx → self.state.xxx
c = c.replace('self.runtime.state.', 'self.state.')

# 5. Replace all self.runtime.mapping.xxx → self.mapping.xxx  
c = c.replace('self.runtime.mapping.', 'self.mapping.')

# 6. Remove current_key field usage (from PipelineState::new)
# Find new() and update
old_new = """    pub fn new(config: Config) -> Self {
        let mut s = PipelineState {
            config,
            tick: 0,
            runtime: Runtime {
                mapping: DeviceMapping {
                    tap_dance: HashMap::new(),
                    combo: ComboIndex::default(),
                    leader: LeaderDef { sequences: vec![], timeout_ms: 1000 },
                },
                state: DeviceState::default(),
            },
            current_device: 0,
            current_key: (0, String::new()),"""
new_new = """    pub fn new(config: Config) -> Self {
        let mut s = PipelineState {
            config,
            tick: 0,
            mapping: Arc::new(DeviceMapping {
                tap_dance: HashMap::new(),
                combo: ComboIndex::default(),
                leader: LeaderDef { sequences: vec![], timeout_ms: 1000 },
            }),
            state: DeviceState::default(),
            current_domain: 0,
            current_device: 0,
            current_app: String::new(),"""
c = c.replace(old_new, new_new)

# 7. Remove Runtime import/usage from cfg(test)
# (The test module may reference Runtime directly)
c = c.replace('use super::Runtime;', '')
c = c.replace('\n    fn build_test_runtime(', '\n    fn _build_test_runtime(')

with open('src/state.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('state.rs updated')
