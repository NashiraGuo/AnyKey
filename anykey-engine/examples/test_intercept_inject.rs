// Minimal intercept+inject test — mimics engine flow.
// Opens driver → enables interception → injects a key → verifies.
// Usage: cargo run --example test_intercept_inject --features filter-driver

use anykey_engine::filter_driver::{FilterDriver, AnyKeyOutputEvent, ANYKEY_KEY_BREAK};
use std::time::Duration;

fn main() {
    println!("=== Intercept + Inject Test ===");

    let mut fd = match FilterDriver::open() {
        Ok(f) => { println!("Driver opened"); f }
        Err(e) => { eprintln!("FAIL: {}", e); return; }
    };

    // Register event for fast poll
    match fd.register_event() {
        Ok(_) => println!("Event registered"),
        Err(e) => { eprintln!("FAIL: {}", e); return; }
    }

    // Enable interception (same as engine)
    match fd.set_device_intercept(0, true) {
        Ok(_) => println!("Interception ON"),
        Err(e) => { eprintln!("FAIL: {}", e); return; }
    }

    println!("\n=== Injecting 'z' (0x2C) 3 times with 5s gap ===");
    for i in 1..=3 {
        let down = AnyKeyOutputEvent { make_code: 0x2C, flags: 0, device_id: 0 };
        match fd.send_output(&down) {
            Ok(_) => println!("  [{}/3] DN z - OK", i),
            Err(e) => eprintln!("  [{}/3] DN z - FAIL: {}", i, e),
        }
        std::thread::sleep(Duration::from_millis(100));

        let up = AnyKeyOutputEvent { make_code: 0x2C, flags: ANYKEY_KEY_BREAK, device_id: 0 };
        match fd.send_output(&up) {
            Ok(_) => println!("  [{}/3] UP z - OK", i),
            Err(e) => eprintln!("  [{}/3] UP z - FAIL: {}", i, e),
        }
        std::thread::sleep(Duration::from_millis(5000));
    }

    // Poll briefly to verify event-driven input still works
    println!("\n=== Quick poll check ===");
    match fd.poll_input_timeout(1000) {
        Ok(events) => println!("Poll returned {} events (expected 0 or few)", events.len()),
        Err(e) => println!("Poll failed: {}", e),
    }

    let _ = fd.set_device_intercept(0, false);
    println!("\nDone. Check Notepad for 'zzz' output.");
}
