//! 便携后端（llhook）的**接线**测试：钩子装上 → 回调被调用 → 事件进通道 →
//! `poll_all` 取到 → 管道映射 → 输出回 SendInput → 自己的输出又回到钩子。
//!
//! # 为什么需要它
//!
//! `hook_input.rs` 的单元测试只覆盖 `project()` / `project_mouse()` / `decide()` 这几个**纯函数**；
//! `backend.rs` 只覆盖 `choose_backend()` 的取值规则。而"钩子到底有没有被调用、事件有没有真的
//! 走到 `poll_all`"这条**接线**，此前只有 `tools/smoke_llhook.py` 验证过 —— 那个脚本需要人按键。
//! 本文件把它自动化。
//!
//! # 为什么这不影响你正在用的键盘
//!
//! 测试用 `HookOptions::for_test()` 安装：`swallow = false` —— 钩子**只观察、不吞任何东西**，
//! 物理按键原样透传（这正是驱动后端已有的「透传捕获模式」）。`decide()` 的真值表里
//! `test_mode_mirrors_everything_and_never_swallows` 是这条安全属性的形式化保证。
//!
//! # 为什么用 F13 / F14 而不是 'a'
//!
//! 测试必须真的注入按键，而注入的键会打到**当前焦点窗口**。F13/F14 在 Windows 上没有任何应用
//! 会响应，注入它们不会在谁的窗口里留下字符、也不会触发动作（'a' 会）。
//!
//! # 一个进程只能装一份钩子
//!
//! `hook_input::SHARED` 是 `OnceLock`，第二次 `install` 返回 Err。所以整个文件**只写一个 test fn**
//! ——`cargo test` 会在同一进程里并行跑同文件的测试，拆成多个必然互相撞。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anykey_engine::config::{Config, KeyEntry, Layers, LeaderConfig};
use anykey_engine::emit::{send_key_down_sendinput, send_key_up_sendinput};
use anykey_engine::events::{
    AnyKeyInputEvent, AnyKeyMouseEvent, AnyKeyMouseOutputEvent, AnyKeyOutputEvent,
    ANYKEY_KEY_BREAK, MOUSE_MOVE_RELATIVE, MOUSE_WHEEL,
};
use anykey_engine::hook_input::{HookInput, HookOptions};
use anykey_engine::sendinput_out::send_mouse_output;
use anykey_engine::state::{EmitEvent, PipelineState};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};

/// 测试键：F13 扫描码（裸扫描码，无 E0 前缀）。F13/F14 在 Windows 上没有应用会响应。
const SCAN_F13: u16 = 0x64;
/// 第 6 步"输出回流"用 F14（另一个无人响应的键）。
const SCAN_F14: u16 = 0x65;
/// 项目里按键名寻址（`pipeline.key_down("f13")`），所以测试自己给这个扫描码配名字。
/// 生产由 `main.rs::input_name()` 负责这个转换（它在 bin 里，集成测试用不到）。
const NAME_F13: &str = "f13";
/// 映射的输出用 `{lshift}`：它**在 scancode 表里**（合法），而且单独按一下 Shift 不产生任何
/// 可见效果、不留任何状态（up 紧随 down）。
///
/// ⚠️ 别用 `{f13}`/`{f14}` 当输出 —— `commit.rs::is_valid_key_name()` 要求 `{X}` 是多字符时
/// 必须能在 scancode / 鼠标键名表里查到，而那两张表只到 f12。写成 `{f14}` 会被**静默丢弃**
/// （映射建好后 resolver 拒绝输出），表现为 `emit_log` 为空、没有任何报错。
const NAME_LSHIFT: &str = "lshift";

/// 在 `ms` 毫秒内持续 `poll_all` 并**累积**结果。
///
/// ⚠️ 必须累积，不能"边轮询边找第一个匹配就返回"：`poll_all` 会把一次批量里的**所有**事件
/// 都取走，如果 down 与 up 落在同一个 100ms 批次里，找到 down 返回就等于把 up 丢了
/// （本测试第一版正是这么挂的）。
///
/// 另外 Mirror 模式会把本机真实输入也镜像进来（用户此刻可能在打字），所以断言一律
/// **按扫描码筛**，不依赖顺序，也不依赖"总共几条"。
fn collect(hook: &HookInput, ms: u64) -> (Vec<AnyKeyInputEvent>, Vec<AnyKeyMouseEvent>) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    let (mut keys, mut mice) = (Vec::new(), Vec::new());
    while Instant::now() < deadline {
        match hook.poll_all(100) {
            Ok((k, m)) => {
                keys.extend(k);
                mice.extend(m);
            }
            Err(e) => panic!("poll_all failed (泵线程死了?): {}", e),
        }
    }
    (keys, mice)
}

/// 注入前先清一次通道：丢掉可能残留在里面的真实用户输入，免得污染后面的判定。
fn drain(hook: &HookInput) {
    let _ = collect(hook, 120);
}

/// 单键 tap 映射的最小配置（`f13` → `{lshift}`）。
fn config_f13_to_lshift() -> Config {
    let mut base = HashMap::new();
    base.insert(NAME_F13.to_string(), KeyEntry { tap: "{lshift}".into(), ..Default::default() });
    let mut layer_maps = HashMap::new();
    layer_maps.insert("base".to_string(), base);
    Config {
        combo_time: 200,
        combo_map: vec![],
        layers: Layers { layer_maps, hold_term: 150, double_tap_term: 250, double_hold_term: 150 },
        leader: LeaderConfig::default(),
        subscribed_devices: vec![],
        app_aware: Default::default(),
        per_device: false,
        devices: HashMap::new(),
    }
}

#[test]
fn llhook_wiring_end_to_end() {
    // ── 1. 安装（只观察，不影响本机键盘）──
    let hook = HookInput::install_with(HookOptions::for_test())
        .expect("install llhook backend (keyboard + mouse hooks)");
    std::thread::sleep(Duration::from_millis(150)); // 让泵线程站稳

    // ── 2. 键盘：注入 F13 down + up，验证 投影 → 通道 → poll_all ──
    drain(&hook);
    let sent_down = send_key_down_sendinput(SCAN_F13, false);
    let sent_up = send_key_up_sendinput(SCAN_F13, false);
    assert!(
        sent_down > 0 && sent_up > 0,
        "SendInput 返回 0（down={} up={}）—— 注入被 UIPI / 安全软件挡住，本机测不了",
        sent_down, sent_up
    );

    let (keys, _) = collect(&hook, 700);
    let down = keys
        .iter()
        .find(|k| k.make_code == SCAN_F13 && k.flags & ANYKEY_KEY_BREAK == 0)
        .unwrap_or_else(|| panic!(
            "钩子没收到 F13 按下 —— 钩子装上≠收得到（见 docs §3.4）；收到的是 {:?}",
            keys.iter().map(|k| (k.make_code, k.flags)).collect::<Vec<_>>()));
    assert_eq!(down.flags, 0, "普通键（非扩展）的 flags 应为 0，实际 0x{:02X}", down.flags);
    assert_eq!(down.device_id, 0, "便携模式全部归到伪设备 0");

    let up = keys
        .iter()
        .find(|k| k.make_code == SCAN_F13 && k.flags & ANYKEY_KEY_BREAK != 0)
        .unwrap_or_else(|| panic!(
            "钩子没收到 F13 抬起；收到的是 {:?}",
            keys.iter().map(|k| (k.make_code, k.flags)).collect::<Vec<_>>()));
    assert_eq!(up.flags & ANYKEY_KEY_BREAK, ANYKEY_KEY_BREAK, "抬起必须带 BREAK 位");

    // ── 3. 鼠标：注入一次滚轮，验证鼠标钩子进的**同一条通道** ──
    // 先把光标挪到 (0,0) 附近：Mirror 模式下滚轮会真的透传给系统，落在桌面/任务栏角落
    // 不会滚动任何文档。用完还原。
    let mut saved = POINT { x: 0, y: 0 };
    let have_saved = unsafe { GetCursorPos(&mut saved) } != 0;
    unsafe { SetCursorPos(4, 4) };
    std::thread::sleep(Duration::from_millis(60));
    drain(&hook);

    let wheel = AnyKeyMouseOutputEvent {
        device_id: 0,
        flags: MOUSE_MOVE_RELATIVE,
        button_flags: MOUSE_WHEEL,
        button_data: 120,
        last_x: 0,
        last_y: 0,
    };
    send_mouse_output(&wheel).expect("send_mouse_output(wheel) 应成功");

    let (_, mice) = collect(&hook, 700);
    let ev = mice
        .iter()
        .find(|m| m.button_flags & MOUSE_WHEEL != 0)
        .unwrap_or_else(|| panic!(
            "鼠标钩子没把滚轮送进通道 —— 键盘/鼠标共用通道这条设计没生效；收到的是 {:?}",
            mice.iter().map(|m| (m.button_flags, m.button_data)).collect::<Vec<_>>()));
    assert_eq!(ev.button_data, 120, "滚轮增量必须保留符号与量值（+120 = 向前）");
    assert_eq!(ev.flags, MOUSE_MOVE_RELATIVE, "按键/滚轮包的 flags 恒为相对移动（与驱动一致）");
    assert_eq!((ev.last_x, ev.last_y), (0, 0), "按键/滚轮包没有位移语义（与驱动一致）");

    if have_saved {
        unsafe { SetCursorPos(saved.x, saved.y) };
    }

    // ── 4. 存活对账：装上之后确实被调用过 ──
    let l = hook.liveness();
    // 注：callbacks/mouse_edges 统计的是**所有**镜像进来的事件，用户此刻打字也会计入，
    // 所以这里只断言"非零"这个下界；精确性由第 2、3 步的扫描码断言负责。
    assert!(l.callbacks > 0, "callbacks 为 0：钩子装上但一次回调都没有");
    assert!(l.mouse_edges > 0, "mouse_edges 为 0：鼠标钩子没被调用");
    eprintln!(
        "[llhook] liveness: kb_callbacks={} mouse_edges={} mouse_moves={} passthrough={} since_install={}ms healthy={}",
        l.callbacks, l.mouse_edges, l.mouse_moves, l.passthrough, l.since_install_ms, l.healthy
    );

    // ── 5. 接上管道：把**从钩子拿到的事件**喂进管道，验证投影与映射约定对得上 ──
    let mut pipeline = PipelineState::new(config_f13_to_lshift());
    let name = match down.make_code {
        SCAN_F13 => NAME_F13,
        other => panic!("期望扫描码 0x{:02X}，实际 0x{:02X}", SCAN_F13, other),
    };
    pipeline.current_device = down.device_id;
    pipeline.key_down(name);
    pipeline.key_up(name);

    assert!(
        pipeline.emit_log.iter().any(|e| matches!(e, EmitEvent::Down(k, _) if k == NAME_LSHIFT)),
        "管道没有把 {} 映射成 {}（emit_log={:?}）—— 注意 is_valid_key_name() 会拒绝表里没有的键名，\
         一旦输出名非法，映射会被静默丢弃、emit_log 为空",
        NAME_F13, NAME_LSHIFT, pipeline.emit_log
    );

    // ── 6. 输出侧：把键真的发出去，并证明"自己的输出会回流" ──
    // 用 F14 的裸扫描码（不经管道取名，故不受 is_valid_key_name 限制），down+up 一对，
    // 不留任何按键状态。F14 无应用响应。
    let out_down = AnyKeyOutputEvent { make_code: SCAN_F14, flags: 0, device_id: 0 };
    let out_up = AnyKeyOutputEvent { make_code: SCAN_F14, flags: ANYKEY_KEY_BREAK, device_id: 0 };
    hook.send_output(&out_down).expect("llhook 输出（SendInput）必须成功");
    hook.send_output(&out_up).expect("llhook 输出的抬起必须成功");

    // 这一步是**关键证据**：本后端输出走 SendInput，它会再次穿过本钩子。生产模式靠
    // accept_injected=false 把它挡在门外（否则自己的输出变成自己的输入 → 死循环）。
    // 测试模式（accept_injected=true）下应当能抓到它 —— 这正是那条过滤存在的理由。
    let (echoed, _) = collect(&hook, 800);
    assert!(
        echoed.iter().any(|k| k.make_code == SCAN_F14),
        "没抓到自己的输出回流（收到 {:?}）：说明 SendInput 注入的键没有再次经过本钩子 —— \
         要么注入被挡，要么钩子已失效。生产模式依赖这次回流被过滤掉，所以这条必须成立",
        echoed.iter().map(|k| (k.make_code, k.flags)).collect::<Vec<_>>()
    );

    // 收尾：卸钩子（Drop 会撤闸门 + 卸钩子 + 让泵线程退出）
    drop(hook);
}
