// test_filter_driver_stability.rs - Comprehensive stability validation
//
// Tests:
//   A. Open/Close cycle (10x)
//   B. Intercept toggle (10x on/off)
//   C. Device info query
//   D. Short poll (15s, verify events received with key press prompt)
//   E. Disconnect/reconnect cycle
//   F. Output injection stress (100 keystrokes)
//   G. Concurrent poll + inject
//
// Build: cargo build --features filter-driver --example test_filter_driver_stability
// Run:   target/debug/examples/test_filter_driver_stability.exe [--duration-sec N]
//        Default N=60 for extended stability mode. Omit for quick mode.

use anykey_engine::filter_driver::{FilterDriver, AnyKeyOutputEvent, ANYKEY_KEY_BREAK};
use std::time::{Duration, Instant};

extern "system" {
    fn CreateFileW(
        lpFileName: *const u16, dwDesiredAccess: u32, dwShareMode: u32,
        lpSecurityAttributes: *const std::ffi::c_void, dwCreationDisposition: u32,
        dwFlagsAndAttributes: u32, hTemplateFile: *const std::ffi::c_void,
    ) -> isize;
}

const GENERIC_READ: u32 = 0x80000000;
const GENERIC_WRITE: u32 = 0x40000000;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

fn open_raw() -> Result<FilterDriver, String> {
    let path: Vec<u16> = "\\\\.\\AnyKeyFlt\0".encode_utf16().collect();
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == -1isize || handle == 0 {
        return Err("CreateFileW failed".to_string());
    }
    Ok(FilterDriver::from_handle(handle as _))
}

/// Open + register event for zero-latency blocking poll.
fn open_with_event() -> Result<FilterDriver, String> {
    let mut fd = open_raw()?;
    fd.register_event()?;
    Ok(fd)
}

#[derive(Default)]
struct Report {
    passed: u32,
    failed: u32,
    lines: Vec<String>,
}

impl Report {
    fn test(&mut self, name: &str, result: Result<(), String>) {
        match result {
            Ok(_) => {
                self.passed += 1;
                self.lines.push(format!("  PASS  {}", name));
            }
            Err(e) => {
                self.failed += 1;
                self.lines.push(format!("  FAIL  {}  ({})", name, e));
            }
        }
    }

    fn info(&mut self, msg: &str) {
        self.lines.push(format!("  INFO  {}", msg));
    }

    fn header(&mut self, h: &str) {
        self.lines.push(format!("\n--- {} ---", h));
    }

    fn print(&self) {
        println!("\n{}", "=".repeat(60));
        for line in &self.lines { println!("{}", line); }
        println!("\n{}", "=".repeat(60));
        println!("Results: {} passed, {} failed, {} total",
                 self.passed, self.failed, self.passed + self.failed);
        if self.failed > 0 {
            println!("SOME TESTS FAILED");
        } else {
            println!("ALL TESTS PASSED");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let duration_sec: u64 = if args.len() >= 3 && args[1] == "--duration-sec" {
        args[2].parse().unwrap_or(60)
    } else {
        0 // quick mode: skip extended stability
    };

    println!("AnyKey Filter Driver - Stability Test Suite");
    println!("{}", "=".repeat(60));
    if duration_sec > 0 {
        println!("Extended mode: {} seconds stability poll", duration_sec);
    } else {
        println!("Quick mode: basic tests only (use --duration-sec N for extended)");
    }

    let mut report = Report::default();

    // ── A. Open/Close cycle ──
    report.header("A. Open/Close cycle (10x)");
    for i in 1..=10 {
        let name = format!("open/close #{}", i);
        let result = (|| -> Result<(), String> {
            let fd = open_raw()?;
            drop(fd);
            Ok(())
        })();
        report.test(&name, result);
        std::thread::sleep(Duration::from_millis(100));
    }

    // ── B. Intercept toggle ──
    report.header("B. Intercept toggle (10x)");
    let fd = match open_raw() {
        Ok(f) => {
            report.info("device opened for toggle tests");
            f
        }
        Err(e) => {
            report.test("open device for toggle", Err(e));
            report.print();
            std::process::exit(1);
        }
    };

    for i in 1..=10 {
        let name = format!("intercept ON #{}", i);
        report.test(&name, fd.set_device_intercept(0, true));
        let name = format!("intercept OFF #{}", i);
        report.test(&name, fd.set_device_intercept(0, false));
    }

    // ── C. Device info ──
    report.header("C. Device info");
    match fd.get_device_count() {
        Ok(n) => report.info(&format!("device count = {}", n)),
        Err(e) => report.test("get_device_count", Err(e)),
    }

    // ── D. Short poll with key prompt (event-driven) ──
    report.header("D. Short poll (15s, verify events, EVENT-DRIVEN)");
    report.info("TURN ON INTERCEPTION, press keys NOW for 15 seconds...");
    let fd_d = match open_with_event() {
        Ok(f) => { report.info("event registered"); f }
        Err(e) => {
            report.test("open+event for D", Err(e));
            report.print();
            std::process::exit(1);
        }
    };
    match fd_d.set_device_intercept(0, true) {
        Ok(_) => {
            let start = Instant::now();
            let duration = Duration::from_secs(15);
            let mut total: u32 = 0;
            while start.elapsed() < duration {
                if let Ok(events) = fd_d.poll_input_blocking() {
                    total += events.len() as u32;
                }
                if start.elapsed() >= duration { break; }
            }
            let _ = fd_d.set_device_intercept(0, false);
            if total > 0 {
                report.test("poll saw events", Ok(()));
                report.info(&format!("total events captured: {}", total));
            } else {
                report.test("poll saw events", Err("0 events captured".to_string()));
            }
        }
        Err(e) => report.test("set_intercept ON for poll", Err(e)),
    }

    // ── E. Disconnect/reconnect ──
    report.header("E. Disconnect/reconnect (5 cycles)");
    drop(fd); // close
    for i in 1..=5 {
        std::thread::sleep(Duration::from_millis(500));
        let name = format!("reconnect #{}", i);
        let result = (|| -> Result<(), String> {
            let fd2 = open_raw()?;
            fd2.get_device_count()?; // verify alive
            drop(fd2);
            Ok(())
        })();
        report.test(&name, result);
    }

    // Re-open for remaining tests
    let fd = match open_raw() {
        Ok(f) => f,
        Err(e) => {
            report.test("reopen for stress tests", Err(e));
            report.print();
            std::process::exit(1);
        }
    };

    // ── F. Output injection stress ──
    report.header("F. Output injection stress (100 keystrokes)");
    // Inject key 'A' (0x1E) 100 times down+up pairs, verify no crash
    let mut inject_ok = true;
    for i in 1..=100 {
        let down = AnyKeyOutputEvent { make_code: 0x1E, flags: 0, device_id: 0 };
        if fd.send_output(&down).is_err() { inject_ok = false; break; }
        let up = AnyKeyOutputEvent { make_code: 0x1E, flags: ANYKEY_KEY_BREAK, device_id: 0 };
        if fd.send_output(&up).is_err() { inject_ok = false; break; }
        if i % 20 == 0 { std::thread::sleep(Duration::from_millis(1)); }
    }
    if inject_ok {
        report.test("inject 100 keystrokes", Ok(()));
    } else {
        report.test("inject 100 keystrokes", Err("send_output failed mid-stream".to_string()));
    }

    // ── G. Concurrent poll + inject ──
    report.header("G. Concurrent poll + inject (10s)");

    // Open injection handle inside the thread to satisfy Send
    let inject_thread = std::thread::spawn(|| {
        let fd2 = match open_raw() {
            Ok(f) => f,
            Err(e) => { eprintln!("inject thread: {}", e); return; }
        };
        for i in 0..200 {
            let code: u16 = 0x1E + (i % 10);
            let down = AnyKeyOutputEvent { make_code: code, flags: 0, device_id: 0 };
            let up = AnyKeyOutputEvent { make_code: code, flags: ANYKEY_KEY_BREAK, device_id: 0 };
            let _ = fd2.send_output(&down);
            let _ = fd2.send_output(&up);
            if i % 10 == 0 { std::thread::sleep(Duration::from_millis(1)); }
        }
    });

    // Poll on main handle concurrently
    match fd.set_device_intercept(0, true) {
        Ok(_) => {
            let start = Instant::now();
            let mut poll_ok = true;
            while start.elapsed() < Duration::from_secs(10) {
                if fd.poll_input().is_err() { poll_ok = false; break; }
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = fd.set_device_intercept(0, false);
            if poll_ok {
                report.test("concurrent poll+inject", Ok(()));
            } else {
                report.test("concurrent poll+inject", Err("poll_input failed under stress".to_string()));
            }
        }
        Err(e) => report.test("set_intercept for concurrent", Err(e)),
    }

    inject_thread.join().ok();

    // ── H. Extended stability (optional) ──
    if duration_sec > 0 {
        report.header(&format!("H. Extended stability ({}s)", duration_sec));
        match fd.set_device_intercept(0, true) {
            Ok(_) => {
                report.info("interception ON, running extended poll...");
                let start = Instant::now();
                let duration = Duration::from_secs(duration_sec);
                let mut total: u32 = 0;
                let mut last_report = Instant::now();
                while start.elapsed() < duration {
                    match fd.poll_input() {
                        Ok(events) => total += events.len() as u32,
                        Err(e) => {
                            report.test("extended stability", Err(format!("poll_input failed at {}s: {}", start.elapsed().as_secs(), e)));
                            break;
                        }
                    }
                    if last_report.elapsed() >= Duration::from_secs(30) {
                        report.info(&format!("  {}s elapsed, {} events so far", start.elapsed().as_secs(), total));
                        last_report = Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                let _ = fd.set_device_intercept(0, false);
                if start.elapsed() >= duration {
                    report.test(&format!("extended stability ({}s)", duration_sec), Ok(()));
                    report.info(&format!("total events: {}", total));
                }
            }
            Err(e) => report.test("set_intercept for extended", Err(e)),
        }
    }

    drop(fd);
    report.print();

    if report.failed > 0 {
        std::process::exit(1);
    }
}
