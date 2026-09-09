// Minimal injection test — NO interception, just open driver and inject.
use anykey_engine::filter_driver::{FilterDriver, AnyKeyOutputEvent, ANYKEY_KEY_BREAK};
use std::time::Duration;

fn main() {
    println!("=== Inject-Only Test (NO interception) ===");
    let fd = match FilterDriver::open() {
        Ok(f) => { println!("Driver opened"); f }
        Err(e) => { eprintln!("FAIL: {}", e); return; }
    };

    println!("\nSwitch to Notepad NOW (3s)...");
    std::thread::sleep(Duration::from_secs(3));

    println!("Injecting 'z' (0x2C)...");
    let down = AnyKeyOutputEvent { make_code: 0x2C, flags: 0, device_id: 0 };
    match fd.send_output(&down) {
        Ok(_) => println!("  DN z - OK"),
        Err(e) => eprintln!("  DN z - FAIL: {}", e),
    }
    std::thread::sleep(Duration::from_millis(100));
    let up = AnyKeyOutputEvent { make_code: 0x2C, flags: ANYKEY_KEY_BREAK, device_id: 0 };
    match fd.send_output(&up) {
        Ok(_) => println!("  UP z - OK"),
        Err(e) => eprintln!("  UP z - FAIL: {}", e),
    }

    println!("\nDone. Check Notepad for 'z'.");
}
