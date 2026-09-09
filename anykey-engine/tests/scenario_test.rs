/// AnyKey engine — Scenario-based differential tests
/// Loads golden data captured from AHK engine, replays against
/// Rust pipeline, and compares emit_log outputs.

use anykey_engine::state::*;
use anykey_engine::config::*;
use serde::Deserialize;
use std::fs;
use std::path::Path;

// ═══════════════════════════════════════════
// JSON scenario types (matches capture_golden.py output)
// ═══════════════════════════════════════════

#[derive(Debug, Deserialize)]
struct TestStep {
    #[serde(default)]
    dn: Option<String>,
    #[serde(default)]
    up: Option<String>,
    #[serde(default)]
    wait: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Scenario {
    name: String,
    desc: String,
    config: Config,
    steps: Vec<TestStep>,
    expect: Vec<serde_json::Value>,
}

// ═══════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════

/// Convert a JSON emit event to string representation for comparison
fn event_to_string(ev: &serde_json::Value) -> String {
    if let Some(obj) = ev.as_object() {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                return format!("{}({})", k, s);
            }
        }
        format!("{:?}", ev)
    } else {
        format!("{:?}", ev)
    }
}

/// Convert EmitEvent to string for diff output
fn emit_to_string(ev: &EmitEvent) -> String {
    match ev {
        EmitEvent::Down(k, _) => format!("Down({})", k),
        EmitEvent::Up(k, _) => format!("Up({})", k),
        EmitEvent::Tap(k, _) => format!("Tap({})", k),
        EmitEvent::MouseDown(k, _) => format!("Down({})", k),
        EmitEvent::MouseUp(k, _) => format!("Up({})", k),
        EmitEvent::Text(t, _) => format!("Text({})", t),
        EmitEvent::Run(c, _) => format!("Run({})", c),
        EmitEvent::MouseMove(x, y, _) => format!("MouseMove({},{})", x, y),
        EmitEvent::LayerOn(l, _) => format!("LayerOn({})", l),
        EmitEvent::LayerOff(l, _) => format!("LayerOff({})", l),
        EmitEvent::TapSI(k, _) => format!("TapSI({})", k),
        EmitEvent::DownSI(k, _) => format!("DownSI({})", k),
        EmitEvent::UpSI(k, _) => format!("UpSI({})", k),
    }
}

/// Get event type name from JSON value
fn json_event_type(ev: &serde_json::Value) -> Option<String> {
    ev.as_object()?.keys().next().cloned()
}

/// Get event key/value from JSON
fn json_event_value(ev: &serde_json::Value) -> Option<String> {
    ev.as_object()?.values().next()?.as_str().map(|s| s.to_string())
}

/// Match AHK event to Rust EmitEvent
fn events_match(ahk: &serde_json::Value, rust: &EmitEvent) -> bool {
    let ev_type = match json_event_type(ahk) {
        Some(t) => t,
        None => return false,
    };
    let ev_val = match json_event_value(ahk) {
        Some(v) => v.to_lowercase(),
        None => return false,
    };

    match ev_type.as_str() {
        "Down" => matches!(rust, EmitEvent::Down(k, _) | EmitEvent::MouseDown(k, _) if k.to_lowercase() == ev_val),
        "Up" => matches!(rust, EmitEvent::Up(k, _) | EmitEvent::MouseUp(k, _) if k.to_lowercase() == ev_val),
        "Tap" => matches!(rust, EmitEvent::Tap(k, _) if k.to_lowercase() == ev_val),
        "Text" => matches!(rust, EmitEvent::Text(t, _) if t.to_lowercase() == ev_val),
        "Run" => matches!(rust, EmitEvent::Run(c, _) if c.to_lowercase() == ev_val),
        "LayerOn" => matches!(rust, EmitEvent::LayerOn(l, _) if l.to_lowercase() == ev_val),
        "LayerOff" => matches!(rust, EmitEvent::LayerOff(l, _) if l.to_lowercase() == ev_val),
        _ => false,
    }
}

// ═══════════════════════════════════════════
// Scenario runner
// ═══════════════════════════════════════════

fn run_scenario(path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("read: {}", e))?;

    let scenario: Scenario = serde_json::from_str(&content)
        .map_err(|e| format!("parse: {}", e))?;

    let mut pipeline = PipelineState::new(scenario.config);
    pipeline.debug_enabled = true;

    // Execute test steps
    for step in &scenario.steps {
        if let Some(ref key) = step.dn {
            pipeline.key_down(key);
        } else if let Some(ref key) = step.up {
            pipeline.key_up(key);
        } else if step.wait.is_some() {
            pipeline.advance_ms(step.wait.unwrap());
        }
    }

    let emit_log = &pipeline.emit_log;
    let expected = &scenario.expect;

    // Write debug log to scenarios directory
    if !pipeline.debug_log.is_empty() {
        let debug_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap()
            .join("tests").join("scenarios")
            .join(format!("{}.debug.log", scenario.name));
        let _ = std::fs::create_dir_all(debug_path.parent().unwrap());
        let _ = fs::write(&debug_path, pipeline.debug_log.join("\n"));
        eprintln!("Debug written to {:?}", debug_path);
    }

    // Compare outputs
    let max_len = emit_log.len().max(expected.len());
    let mut errors = vec![];

    for i in 0..max_len {
        let rust_ev = emit_log.get(i);
        let ahk_ev = expected.get(i);

        match (rust_ev, ahk_ev) {
            (Some(r), Some(a)) => {
                if !events_match(a, r) {
                    errors.push(format!(
                        "  [{}] mismatch:\n    AHK:  {}\n    Rust: {}",
                        i, event_to_string(a), emit_to_string(r)
                    ));
                }
            }
            (Some(r), None) => {
                errors.push(format!(
                    "  [{}] extra Rust event: {}",
                    i, emit_to_string(r)
                ));
            }
            (None, Some(a)) => {
                errors.push(format!(
                    "  [{}] missing Rust event, AHK has: {}",
                    i, event_to_string(a)
                ));
            }
            (None, None) => break,
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        let debug_tail: Vec<&str> = pipeline.debug_log.iter()
            .map(|s| s.as_str())
            .collect();
        Err(format!(
            "{} FAILED (expect={}, got={}):\n  desc: {}\n{}\n--- Debug log (last 15) ---\n{}",
            scenario.name,
            expected.len(),
            emit_log.len(),
            scenario.desc,
            errors.join("\n"),
            debug_tail.iter().rev().take(15).rev()
                .map(|s| format!("  {}", s))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

// ═══════════════════════════════════════════
// Generated test functions
// ═══════════════════════════════════════════

/// Discover scenario files and run them
#[test]
fn test_all_scenarios() {
    let scenarios_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .join("tests").join("scenarios");

    if !scenarios_dir.exists() {
        println!("Scenario dir not found: {:?}, skipping", scenarios_dir);
        return;
    }

    let mut entries: Vec<_> = fs::read_dir(&scenarios_dir)
        .expect("read scenarios dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "json"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut errors = vec![];

    for entry in &entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        match run_scenario(&path) {
            Ok(()) => {
                passed += 1;
                println!("  PASS: {}", name);
            }
            Err(msg) => {
                failed += 1;
                println!("  FAIL: {}", name);
                errors.push(msg);
            }
        }
    }

    if failed > 0 {
        panic!(
            "\n{} scenarios: {} passed, {} failed\n\n{}",
            entries.len(),
            passed,
            failed,
            errors.join("\n")
        );
    }

    println!("\nAll {} scenarios passed!", passed);
}
