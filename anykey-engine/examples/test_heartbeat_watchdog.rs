//! Heartbeat watchdog trigger test — physical keyboard version.
//!
//! Verifies: when engine hangs (heartbeat stops), watchdog disables
//! interception and restores pass-through within 30s.
//!
//! Usage (admin): test_heartbeat_watchdog.exe
//!
//! Test phases (user must be at the keyboard):
//!   Phase 1 — Block:         intercept ON → can't type → press keys to confirm
//!   Phase 2 — Kill heartbeat: stop sending heartbeats
//!   Phase 3 — Countdown:     30s countdown → watchdog should fire
//!   Phase 4 — Recover:       try typing → should work (pass-through restored)

use anykey_engine::filter_driver::FilterDriver;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use std::io::{self, Write};

fn main() {
    println!("=== Heartbeat Watchdog Test (Keyboard) ===\n");

    let fd = match FilterDriver::open() {
        Ok(f) => f,
        Err(e) => { eprintln!("[FAIL] {}", e); return; }
    };

    // ═══ Phase 1: Block keyboard ═══
    println!("── Phase 1: Enabling interception ──");
    match fd.set_device_intercept(0, true) {
        Ok(_) => println!("[OK] Interception ON — keyboard is now BLOCKED"),
        Err(e) => { eprintln!("[FAIL] {}", e); return; }
    }

    // Start heartbeat to keep session alive
    let hb_running = Arc::new(AtomicBool::new(true));
    {
        let running = hb_running.clone();
        std::thread::spawn(move || {
            let hb = match FilterDriver::open() { Ok(f) => f, Err(_) => return };
            while running.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(5));
                if !running.load(Ordering::Relaxed) { break; }
                let _ = hb.heartbeat();
            }
            // Leak handle — don't let Drop fire set_intercept(false).
            // The process exit will close all handles anyway.
            std::mem::forget(hb);
        });
    }

    println!("\n  TRY TYPING NOW in Notepad — nothing should appear.");
    print!("  Press Enter when confirmed keyboard is blocked... ");
    io::stdout().flush().ok();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).ok();

    // ═══ Phase 2: Kill heartbeat ═══
    println!("\n── Phase 2: STOPPING heartbeat (simulating engine hang) ──");
    hb_running.store(false, Ordering::Relaxed);
    println!("  Heartbeat thread killed. Watchdog will fire in ~30s.");

    // ═══ Phase 3: Wait for watchdog ═══
    println!("\n── Phase 3: Waiting for watchdog (30s timeout) ──");
    for sec in (1..=35).rev() {
        let marker = if sec == 30 { " ← watchdog may fire now" }
                else if sec == 25 { "" }
                else if sec == 20 { "" }
                else if sec == 15 { "" }
                else if sec == 10 { "" }
                else if sec == 5  { "" }
                else { "" };
        println!("  {}s remaining{}", sec, marker);
        std::thread::sleep(Duration::from_secs(1));
    }

    // ═══ Phase 4: Verify recovery ═══
    println!("\n── Phase 4: TRY TYPING NOW ──");
    println!("  Keyboard should respond normally (pass-through restored).");
    println!("  If stuck = FAIL, if works = PASS.\n");

    print!("  Type 'pass' or 'fail' to record result: ");
    io::stdout().flush().ok();
    let mut result = String::new();
    io::stdin().read_line(&mut result).ok();

    if result.trim().eq_ignore_ascii_case("pass") {
        println!("\n[PASS] Watchdog correctly restored pass-through after heartbeat loss.");
    } else {
        println!("\n[FAIL] Watchdog did NOT restore pass-through. Check driver debug output.");
    }

    // Cleanup
    let _ = fd.set_device_intercept(0, false);
    println!("[DONE] Interception disabled, exiting.");
}
