// test_filter_driver.rs - Minimal validation of the AnyKey filter driver
// RawPDO interface: open device -> set_intercept ON -> poll input -> print events.
//
// Build: cargo build --features filter-driver --example test_filter_driver
// Run:   target/debug/examples/test_filter_driver.exe (as Administrator on the VM)
//
// The driver starts with interception OFF and stays off after this test exits
// (FilterDriver::drop calls CloseHandle, the watchdog auto-disables).

use anykey_engine::filter_driver::{FilterDriver, AnyKeyOutputEvent, ANYKEY_KEY_BREAK};
use std::io::{self, Write};

extern "system" {
    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: u32,
        dwShareMode: u32,
        lpSecurityAttributes: *const std::ffi::c_void,
        dwCreationDisposition: u32,
        dwFlagsAndAttributes: u32,
        hTemplateFile: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
}

const GENERIC_READ: u32 = 0x80000000;
const GENERIC_WRITE: u32 = 0x40000000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

fn main() {
    println!("AnyKey Filter Driver - Standalone Test");
    println!("======================================\n");

    // 1. Open the control device via symbolic link \\.\AnyKeyFlt
    let path: Vec<u16> = "\\\\.\\AnyKeyFlt\0".encode_utf16().collect();
    print!("Opening \\\\.\\AnyKeyFlt... ");
    io::stdout().flush().ok();
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle == -1isize as _ {
        println!("FAILED (CreateFileW returned null/invalid)");
        println!("\nMake sure:");
        println!("  * anykey_flt.sys is installed and the VM has been rebooted");
        println!("  * you are running as Administrator");
        println!("  * WinDbg shows 'Control device created'");
        std::process::exit(1);
    }
    println!("OK");
    let fd = FilterDriver::from_handle(handle as _);

    // 2. Query device count
    match fd.get_device_count() {
        Ok(n) => println!("Device count: {} keyboard(s) filtered", n),
        Err(e) => println!("WARN: get_device_count failed: {}", e),
    }

    // 3. Enable interception
    print!("Enabling interception... ");
    io::stdout().flush().ok();
    match fd.set_device_intercept(0, true) {
        Ok(_) => println!("ON (keys will be captured, NOT delivered to the system)"),
        Err(e) => {
            println!("FAILED: {}", e);
            std::process::exit(1);
        }
    }

    // 4. Poll input for N seconds, print every captured event
    let duration = std::time::Duration::from_secs(15);
    println!("\nPolling input for {} seconds. PRESS KEYS NOW.", duration.as_secs());
    println!("(Keys will be captured by the driver, not shown on screen.)\n");

    let start = std::time::Instant::now();
    let mut total: u32 = 0;

    while start.elapsed() < duration {
        match fd.poll_input() {
            Ok(events) => {
                for evt in &events {
                    let dir = if (evt.flags & ANYKEY_KEY_BREAK) == 0 { "DOWN" } else { "UP  " };
                    let e0 = if (evt.flags & 0x02) != 0 { " E0" } else { "" };
                    let e1 = if (evt.flags & 0x04) != 0 { " E1" } else { "" };
                    println!("  [{:5}] {} code=0x{:02X}{}{} dev={}",
                        total + 1, dir, evt.make_code, e0, e1, evt.device_id);
                    total += 1;
                }
            }
            Err(e) => {
                println!("  poll_input error: {}", e);
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    println!("\nTotal events captured: {}", total);
    if total == 0 {
        println!("WARN: No events captured. Check:");
        println!("  * Does the filter see CONNECT hooked in WinDbg?");
        println!("  * Is interception actually ON? (service watchdog: any polling keeps it alive)");
        println!("  * Try pressing keys on a DIFFERENT keyboard (e.g., Basic vs Enhanced session).");
    }

    // 5. Disable interception
    print!("Disabling interception... ");
    io::stdout().flush().ok();
    match fd.set_device_intercept(0, false) {
        Ok(_) => {
            println!("OFF (keyboard back to normal)");
        }
        Err(e) => {
            println!("FAILED: {}", e);
        }
    }

    // 6. Output injection test
    println!("\n--- Output Injection Test ---");
    println!("Open Notepad now. The driver will type 'HELLO' in 2 seconds...");
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Scan codes for H E L L O (PS/2 Set 1, no E0 prefix)
    let test_keys: [(u16, &str); 5] = [
        (0x23, "H"), (0x12, "E"), (0x26, "L"), (0x26, "L"), (0x18, "O"),
    ];

    for (code, label) in &test_keys {
        // Key down
        let down = AnyKeyOutputEvent { make_code: *code, flags: 0, device_id: 0 };
        match fd.send_output(&down) {
            Ok(_) => print!("{}", label),
            Err(e) => { println!("FAIL({}): {}", label, e); break; }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Key up
        let up = AnyKeyOutputEvent {
            make_code: *code, flags: ANYKEY_KEY_BREAK, device_id: 0,
        };
        let _ = fd.send_output(&up);
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    println!("\nInjection done. Check Notepad for 'HELLO'.");

    println!("\nTest complete.");
}
