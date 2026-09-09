# Runtime v3 step 4: pipeline.rs — schedule_timer + timer dispatch

with open('src/pipeline.rs', 'r', encoding='utf-8') as f:
    c = f.read()

# Add Arc import
c = c.replace('use crate::state::{PipelineState, TimerEntry, TimerKind,', 
              'use std::sync::Arc;\nuse crate::state::{PipelineState, TimerEntry, TimerKind,')

# Fix schedule_timer — capture mapping snapshot
old_sched = """    pub fn schedule_timer(&mut self, kind: TimerKind, key: &str, td_key: &str, extra_data: &str, delay_ms: u64) {
        self.timer_counter += 1;
        let id = self.timer_counter;
        let entry = TimerEntry {
            id,
            deadline_tick: self.tick + delay_ms,
            kind,
            key: key.to_string(),
            td_key: td_key.to_string(),
            extra_data: extra_data.to_string(),
            device_id: self.current_device,
        };"""
new_sched = """    pub fn schedule_timer(&mut self, kind: TimerKind, key: &str, td_key: &str, extra_data: &str, delay_ms: u64) {
        self.timer_counter += 1;
        let id = self.timer_counter;
        let entry = TimerEntry {
            id,
            deadline_tick: self.tick + delay_ms,
            kind,
            key: key.to_string(),
            td_key: td_key.to_string(),
            extra_data: extra_data.to_string(),
            device_id: self.current_device,
            mapping: self.mapping.clone(),  // v3: 冻结调度时的映射上下文
        };"""
c = c.replace(old_sched, new_sched)

# Fix drain timers — swap mapping on timer dispatch
old_drain = """            if !stale {
                break;  // timer_heap 按 deadline_tick 排序，后续都不会到期
            }
            // 处理该定时器
            self.handle_timer(entry.kind, &entry.td_key, &entry.extra_data, entry.device_id);"""
new_drain = """            if !stale {
                break;  // timer_heap 按 deadline_tick 排序，后续都不会到期
            }
            // 处理该定时器，临时 swap 到调度时的映射快照
            let saved_mapping = self.mapping.clone();
            self.mapping = entry.mapping.clone();
            self.handle_timer(entry.kind, &entry.td_key, &entry.extra_data, entry.device_id);
            self.mapping = saved_mapping;  // 还原"""
c = c.replace(old_drain, new_drain)

with open('src/pipeline.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('pipeline.rs updated')
