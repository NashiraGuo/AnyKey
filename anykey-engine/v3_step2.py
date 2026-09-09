# Runtime v3 step 2: TimerEntry + Arc in state.rs

with open('src/state.rs', 'r', encoding='utf-8') as f:
    c = f.read()

# Add Arc<DeviceMapping> to TimerEntry, remove device_id
old_timer = """pub struct TimerEntry {
    /// 唯一 id：用于「延迟删除」精确取消——取消时记录具体 id，
    /// 新调度条目拿全新 id，天然不受旧取消标记影响（修复「新建计时器复活旧计时器」bug）。
    pub id: TimerId,
    pub deadline_tick: u64,
    pub kind: TimerKind,
    pub key: String,
    pub td_key: String,
    pub extra_data: String,
    /// 调度该定时器的设备 ID（供回调入口设置输出目标设备）。
    pub device_id: u32,
}"""
new_timer = """pub struct TimerEntry {
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
}"""
c = c.replace(old_timer, new_timer)

with open('src/state.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('TimerEntry updated')
