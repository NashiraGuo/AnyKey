//! DeviceDescriptor and Registry — device discovery and hotplug.
//!
//! Registry owns the device scan. It is output-only to Matcher.

use crate::filter_driver::{FilterDriver, AnyKeyEnumDevicesRequest, AnyKeyDeviceInfo};
use std::collections::BTreeMap;

// ── DeviceDescriptor — unified device identity (drv → registry, all modules use) ──

#[derive(Debug, Clone)]
pub struct DeviceDescriptor {
    pub runtime_device_id: u32,
    pub container_id:      Option<String>,
    pub hardware_id:       String,
    pub friendly_name:     String,
    pub vendor_id:         u16,
    pub product_id:        u16,
    pub alias:             Option<String>,
    pub is_keyboard:       bool,
    pub is_mouse:          bool,
    pub has_serial:        bool,
    pub flags:             u32,
}

impl DeviceDescriptor {
    fn from_driver_info(info: &AnyKeyDeviceInfo) -> Self {
        DeviceDescriptor {
            runtime_device_id: info.device_id,
            container_id:      wide_option(&info.container_id),
            hardware_id:       wide_trim(&info.hardware_id),
            friendly_name:     wide_trim(&info.friendly_name),
            vendor_id:         info.vendor_id,
            product_id:        info.product_id,
            alias:             None,
            is_keyboard:       info.is_keyboard != 0,
            is_mouse:          info.is_mouse != 0,
            has_serial:        info.has_serial_nr != 0,
            flags:             info.flags,
        }
    }
}

fn wide_trim(wide: &[u16]) -> String {
    wide.iter()
        .take_while(|&&c| c != 0)
        .map(|&c| char::from_u32(c as u32).unwrap_or('?'))
        .collect()
}

fn wide_option(wide: &[u16]) -> Option<String> {
    let s = wide_trim(wide);
    if s.is_empty() { None } else { Some(s) }
}

// ── Registry ──

pub struct Registry {
    pub descriptors: BTreeMap<u32, DeviceDescriptor>,
}

impl Registry {
    pub fn new() -> Self {
        Registry { descriptors: BTreeMap::new() }
    }

    /// Full scan via driver IOCTL.
    pub fn scan_all(fd: &FilterDriver) -> Result<Vec<DeviceDescriptor>, String> {
        let mut list = Vec::new();
        let mut index: u32 = 0;
        let mut buf: Vec<AnyKeyDeviceInfo> = vec![unsafe { std::mem::zeroed() }; 16];

        loop {
            let req = AnyKeyEnumDevicesRequest { index, max_count: 16 };
            let count = fd.enum_devices(&req, &mut buf)?;
            if count == 0 { break; }
            for i in 0..count as usize {
                list.push(DeviceDescriptor::from_driver_info(&buf[i]));
            }
            index += count;
        }
        Ok(list)
    }

    /// Initialize from driver scan.
    pub fn init(&mut self, fd: &FilterDriver) -> Result<(), String> {
        let list = Self::scan_all(fd)?;
        self.descriptors.clear();
        for d in list {
            self.descriptors.insert(d.runtime_device_id, d);
        }
        Ok(())
    }

    /// Lookup single descriptor.
    pub fn get(&self, id: u32) -> Option<&DeviceDescriptor> {
        self.descriptors.get(&id)
    }

    /// Refresh after hotplug — returns changed/absent devices.
    pub fn refresh(&mut self, fd: &FilterDriver) -> Result<Vec<DeviceChange>, String> {
        let current = Self::scan_all(fd)?;
        let mut changes = Vec::new();

        // Detect removals
        let old_ids: std::collections::HashSet<u32> = self.descriptors.keys().copied().collect();
        let new_ids: std::collections::HashSet<u32> = current.iter().map(|d| d.runtime_device_id).collect();
        for &id in old_ids.difference(&new_ids) {
            changes.push(DeviceChange { old_id: Some(id), new_id: None });
        }
        for &id in new_ids.difference(&old_ids) {
            changes.push(DeviceChange { old_id: None, new_id: Some(id) });
        }

        self.descriptors.clear();
        for d in current {
            self.descriptors.insert(d.runtime_device_id, d);
        }
        Ok(changes)
    }
}

pub struct DeviceChange {
    pub old_id: Option<u32>,
    pub new_id: Option<u32>,
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_descriptors() -> Vec<DeviceDescriptor> {
        vec![
            DeviceDescriptor {
                runtime_device_id: 1,
                container_id: Some("{11111111-1111-1111-1111-111111111111}".into()),
                hardware_id: "HID\\VID_046D&PID_C539&...".into(),
                friendly_name: "Logitech G Pro X".into(),
                vendor_id: 0x046D, product_id: 0xC539,
                alias: Some("主力键盘".into()),
                is_keyboard: true, is_mouse: false,
                has_serial: true, flags: 0,
            },
            DeviceDescriptor {
                runtime_device_id: 2,
                container_id: None,
                hardware_id: "HID\\VID_046D&PID_C539&...".into(),
                friendly_name: "Logitech G Pro X #2".into(),
                vendor_id: 0x046D, product_id: 0xC539,
                alias: Some("副键盘".into()),
                is_keyboard: true, is_mouse: false,
                has_serial: false, flags: 0,
            },
            DeviceDescriptor {
                runtime_device_id: 3,
                container_id: Some("{22222222-2222-2222-2222-222222222222}".into()),
                hardware_id: "HID\\VID_045E&PID_00CB&...".into(),
                friendly_name: "Microsoft Mouse".into(),
                vendor_id: 0x045E, product_id: 0x00CB,
                alias: None,
                is_keyboard: false, is_mouse: true,
                has_serial: true, flags: 0,
            },
        ]
    }

    #[test]
    fn test_descriptor_from_fake_data() {
        let devs = fake_descriptors();
        assert_eq!(devs.len(), 3);
        assert_eq!(devs[0].runtime_device_id, 1);
        assert!(devs[0].has_serial);
        assert!(!devs[1].has_serial);
    }
}
