use anykey_engine::config::Config;
use anykey_engine::state::PipelineState;
use anykey_engine::runtime_builder::build_combo_index;

fn main() {
    let json = std::fs::read_to_string("../anykey_config.json").unwrap();
    let cfg: Config = serde_json::from_str(&json).unwrap();

    // 1) 打印 map 结构（用户要求的 debug）
    let idx = build_combo_index(&cfg.combo_map);
    println!("===== COMBO MAP =====");
    for (layer, lm) in &idx.map {
        for (k, partners) in lm {
            for (p, out) in partners {
                println!("  [{}] {} + {} -> {}", layer, k, p, out);
            }
        }
    }
    println!("  key_set: {:?}", idx.key_set.iter().collect::<Vec<_>>());

    // 2) 模拟 a 按下 7ms 后 s 按下，完整 debug
    let mut s = PipelineState::new(cfg);
    s.debug_enabled = true;
    s.key_down("a");
    s.advance_ms(7);
    s.key_down("s");
    s.advance_ms(80);
    s.key_up("s");
    s.advance_ms(30);
    s.key_up("a");
    s.advance_ms(30);
    println!("===== EMIT =====");
    println!("{:?}", s.emit_log);
}
