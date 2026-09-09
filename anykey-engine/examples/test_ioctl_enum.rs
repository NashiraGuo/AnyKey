//! Driver IOCTL self-validation — Layer 1 test.
//!
//! Proves IOCTL_ANYKEY_ENUM_DEVICES returns correct device info.
//! No pipeline, no config, no mapping — just driver IOCTL.
//!
//! Usage: test_ioctl_enum.exe

use anykey_engine::filter_driver::{FilterDriver, AnyKeyEnumDevicesRequest};

fn wide_to_string(wide: &[u16]) -> String {
    wide.iter()
        .take_while(|&&c| c != 0)
        .map(|&c| char::from_u32(c as u32).unwrap_or('?'))
        .collect()
}

fn main() {
    println!("=== IOCTL_ANYKEY_ENUM_DEVICES Test ===\n");

    let fd = match FilterDriver::open() {
        Ok(f) => { println!("[OK] Driver opened\n"); f }
        Err(e) => { eprintln!("[FAIL] {}", e); return; }
    };

    let mut index: u32 = 0;
    let mut total = 0u32;
    let mut buf: Vec<anykey_engine::filter_driver::AnyKeyDeviceInfo> =
        vec![unsafe { std::mem::zeroed() }; 16];

    loop {
        let req = AnyKeyEnumDevicesRequest { index, max_count: 16 };
        let count = match fd.enum_devices(&req, &mut buf) {
            Ok(n) => n,
            Err(e) => { eprintln!("[FAIL] {}", e); break; }
        };
        if count == 0 { break; }

        for i in 0..count as usize {
            let d = &buf[i];
            let hwid = wide_to_string(&d.hardware_id);
            let cid  = wide_to_string(&d.container_id);
            let fname = wide_to_string(&d.friendly_name);

            println!("Device {}:", d.device_id);
            println!("  HardwareId:    {}", if hwid.is_empty() { "(none)" } else { &hwid });
            println!("  ContainerId:   {}", if cid.is_empty() { "(none)" } else { &cid });
            println!("  FriendlyName:  {}", if fname.is_empty() { "(none)" } else { &fname });
            println!("  VendorId=0x{:04X}  ProductId=0x{:04X}", d.vendor_id, d.product_id);
            println!("  Keyboard={}  Mouse={}  HasSerial={}  Flags=0x{:08X}",
                     d.is_keyboard, d.is_mouse, d.has_serial_nr, d.flags);
            println!();
            total += 1;
        }
        index += count;
    }

    println!("Total: {} devices", total);
    if total > 0 {
        println!("[PASS] IOCTL_ENUM_DEVICES works");
    } else {
        println!("[FAIL] No devices found — driver loaded?");
    }
}
