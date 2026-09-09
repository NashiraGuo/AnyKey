//! Heartbeat idle test: verifies Session+Heartbeat survives 30s of no input.
//!
//! Old watchdog: 30s without WAIT_INPUT → interception killed.
//! New heartbeat: dedicated thread sends HEARTBEAT every 5s → survives idle.
//!
//! Usage (admin): test_heartbeat_idle.exe
//!
//! Steps:
//!   1. Open driver, enable intercept, start heartbeat thread
//!   2. Sleep 35s (longer than the 30s timeout)
//!   3. Inject 'x' to verify interception is still alive
//!   4. Disable intercept, close
//!
//! Pass: 'x' appears in Notepad after 35s idle.
//! Fail: 'x' does NOT appear (interception was killed).

use anykey_engine::filter_driver::{
    ANYKEY_KEY_BREAK, FilterDriver, AnyKeyOutputEvent,
};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

fn main() {
    println!("=== Heartbeat Idle Test ===");
    println!("Expected: AFTER 35s idle pause, 'x' appears in Notepad.\n");

    // 1 ── Open driver
    let fd = match FilterDriver::open() {
        Ok(f) => { println!("[OK] Driver opened"); f }
        Err(e) => { eprintln!("[FAIL] {}", e); return; }
    };

    // 2 ── Enable interception
    match fd.set_device_intercept(0, true) {
        Ok(_) => println!("[OK] Interception ON"),
        Err(e) => { eprintln!("[FAIL] {}", e); return; }
    }

    // 3 ── Start heartbeat thread (same as engine: separate handle, 5s cycle)
    let heartbeat_running = Arc::new(AtomicBool::new(true));
    {
        let running = heartbeat_running.clone();
        std::thread::spawn(move || {
            let hb_fd = match FilterDriver::open() {
                Ok(f) => f,
                Err(_) => return,
            };
            let mut beat = 0u32;
            while running.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(5));
                if !running.load(Ordering::Relaxed) { break; }
                beat += 1;
                match hb_fd.heartbeat() {
                    Ok(resp) => println!("  [HB #{}] driver={:08x} devices={} queue={} state={}",
                        beat, resp.driver_version, resp.device_count,
                        resp.queue_depth, resp.state_flags),
                    Err(e) => println!("  [HB #{}] FAIL: {}", beat, e),
                }
            }
            // Leak handle — don't let Drop fire set_intercept(false)
            std::mem::forget(hb_fd);
        });
    }

    // 4 ── Idle pause: 35 seconds, no keys pressed
    println!("\n  Idle pause: 35 seconds (longer than 30s timeout)...");
    println!("  Switch to Notepad now!");
    for sec in (1..=35).rev() {
        if sec % 5 == 0 || sec <= 5 {
            println!("  ... {}s remaining", sec);
        }
        std::thread::sleep(Duration::from_secs(1));
    }

    // 5 ── Inject 'x' to prove engine is still alive
    println!("\n  Injecting 'x' — should appear in Notepad...");
    let down = AnyKeyOutputEvent { make_code: 0x2D, flags: 0, device_id: 0 };
    let up   = AnyKeyOutputEvent { make_code: 0x2D, flags: ANYKEY_KEY_BREAK, device_id: 0 };

    match fd.send_output(&down) {
        Ok(_) => println!("  [OK] DN x"),
        Err(e) => println!("  [FAIL] DN x: {}", e),
    }
    std::thread::sleep(Duration::from_millis(100));
    match fd.send_output(&up) {
        Ok(_) => println!("  [OK] UP x"),
        Err(e) => println!("  [FAIL] UP x: {}", e),
    }

    // 6 ── Cleanup
    heartbeat_running.store(false, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(200));
    let _ = fd.set_device_intercept(0, false);
    println!("\n[DONE] Check Notepad for 'x'. No 'x' = FAIL.");
}
