// Inject to each device separately to find which one works.
use anykey_engine::filter_driver::{FilterDriver, AnyKeyOutputEvent, ANYKEY_KEY_BREAK};
use std::time::Duration;

fn main() {
    println!("=== Per-Device Inject Test ===");
    let fd = match FilterDriver::open() {
        Ok(f) => { println!("Driver opened"); f }
        Err(e) => { eprintln!("FAIL: {}", e); return; }
    };
    let count = fd.get_device_count().unwrap_or(0);
    println!("Devices: {}", count);

    for dev_id in 2..=8 {  // DeviceIds start at 2, try all possible
        println!("\n--- Device {}: injecting 'x' (0x2D) ---", dev_id);
        println!("Switch to Notepad (3s)...");
        std::thread::sleep(Duration::from_secs(3));

        let down = AnyKeyOutputEvent { make_code: 0x2D, flags: 0, device_id: dev_id };
        match fd.send_output(&down) {
            Ok(_) => println!("  DN x -> dev {} OK", dev_id),
            Err(e) => eprintln!("  DN x -> dev {} FAIL: {}", dev_id, e),
        }
        std::thread::sleep(Duration::from_millis(100));
        let up = AnyKeyOutputEvent { make_code: 0x2D, flags: ANYKEY_KEY_BREAK, device_id: dev_id };
        match fd.send_output(&up) {
            Ok(_) => println!("  UP x -> dev {} OK", dev_id),
            Err(e) => eprintln!("  UP x -> dev {} FAIL: {}", dev_id, e),
        }
        println!("  result: check Notepad for 'x'");
    }
    println!("\nDone.");
}
