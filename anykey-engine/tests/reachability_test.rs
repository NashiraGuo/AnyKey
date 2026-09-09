/// AnyKey engine — 可达性探针测试
/// 目标：验证 single_key_timeout 内 `if self.defer.keys_deferred.contains_key(key)` 分支
/// （pipeline.rs ~1418，用户原注"这段可能是死代码"）是否可达。
///
/// 构造：l 为层切换键(hold {fn1})，k 为 combo 键(搭档 m 不按下)。
/// 预期（用户假设）：k 在 phase3 被 SILENCE(含 F_DEFER) → phase5 defer 被跳过
///   → k 不会进入 keys_deferred → 超时触发时该分支不可达。
/// 若 debug_log 出现 "BRANCH HIT" 则说明该分支实际可达，测试会失败报警。

use anykey_engine::config::Config;
use anykey_engine::state::*;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct ProbeStep {
    #[serde(default)]
    dn: Option<String>,
    #[serde(default)]
    up: Option<String>,
    #[serde(default)]
    wait: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Probe {
    name: String,
    config: Config,
    steps: Vec<ProbeStep>,
}

fn probe_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("probe")
        .join("defer_timeout_probe.json")
}

#[test]
fn probe_deferred_timeout_branch_reachability() {
    let content = fs::read_to_string(probe_path()).expect("read probe json");
    let probe: Probe = serde_json::from_str(&content).expect("parse probe json");

    let mut pipeline = PipelineState::new(probe.config);
    pipeline.debug_enabled = true;

    for step in &probe.steps {
        if let Some(ref key) = step.dn {
            pipeline.key_down(key);
        } else if let Some(ref key) = step.up {
            pipeline.key_up(key);
        } else if let Some(ms) = step.wait {
            pipeline.advance_ms(ms);
        }
    }

    // 打印全部 DEADCODE 探针日志，方便人工查看
    let probe_lines: Vec<&String> = pipeline
        .debug_log
        .iter()
        .filter(|l| l.contains("DEADCODE"))
        .collect();
    println!("--- DEADCODE probe log for '{}' ---", probe.name);
    for line in &probe_lines {
        println!("  {}", line);
    }
    println!("--- end probe log ---");

    let branch_hit = pipeline
        .debug_log
        .iter()
        .any(|l| l.contains("BRANCH HIT"));

    let timeout_probe_lines: Vec<&String> = pipeline
        .debug_log
        .iter()
        .filter(|l| l.contains("TimeoutProbe"))
        .collect();

    assert!(
        !branch_hit,
        "可达性假设被推翻：single_key_timeout 的 keys_deferred 分支实际可达！\n\
         TimeoutProbe 记录：\n{}\n\
         该分支不是死代码，需要重新审视 1418-1428 的逻辑。",
        timeout_probe_lines
            .iter()
            .map(|s| format!("  {}", s))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // 正向确认：确实产生了一次 TimeoutProbe（说明 single_key_timeout 被触发），
    // 且其 inKeysDeferred 应为 false（否则上面的 assert 已失败）
    assert!(
        !timeout_probe_lines.is_empty(),
        "single_key_timeout 未被触发，探针未覆盖目标路径（检查 comboTime/步骤）"
    );
    for line in &timeout_probe_lines {
        assert!(
            line.contains("inKeysDeferred=false"),
            "意外的状态：{} —— combo 超时键竟然落在 keys_deferred 中",
            line
        );
    }
}
