//! Linux: reads sysfs, which every distribution provides. Root access is needed to read the
//! machine's own serial number; without it the computer is reported without one.

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::device::Device;

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Parses the computer from /sys/class/dmi/id-style files under `root`.
pub fn computer(root: &Path, hostname: Option<String>) -> Option<Device> {
    let dmi = root.join("sys/class/dmi/id");
    let vendor = read(&dmi.join("sys_vendor"));
    let product = read(&dmi.join("product_name"));
    if vendor.is_none() && product.is_none() {
        return None;
    }
    let mut d = Device::new("computer", "computer", hostname.unwrap_or_else(|| "This computer".into()));
    d.manufacturer = vendor;
    d.model = product;
    d.serial = read(&dmi.join("product_serial"));
    let chassis = read(&dmi.join("chassis_type")).and_then(|c| c.parse::<u32>().ok());
    let form = match chassis {
        Some(8 | 9 | 10 | 14 | 30 | 31 | 32) => "laptop",
        Some(_) => "desktop",
        None => "unknown",
    };
    d.details.insert("formFactor".into(), json!(form));
    Some(d)
}

/// USB devices from /sys/bus/usb/devices under `root`. Hubs (class 09) are skipped.
pub fn usb(root: &Path) -> Vec<Device> {
    let mut out = vec![];
    let Ok(entries) = fs::read_dir(root.join("sys/bus/usb/devices")) else { return out };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        let (Some(vid), Some(pid)) = (read(&p.join("idVendor")), read(&p.join("idProduct"))) else { continue };
        if read(&p.join("bDeviceClass")).as_deref() == Some("09") || vid == "1d6b" {
            continue;
        }
        let name = read(&p.join("product")).unwrap_or_else(|| format!("USB device {vid}:{pid}"));
        let port = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let serial = read(&p.join("serial"));
        // Without a serial, the port it is plugged into is the best local id there is.
        let local = format!("usb:{vid}:{pid}:{}", serial.clone().unwrap_or(port));
        let mut d = Device::new(local, "peripheral", name);
        d.manufacturer = read(&p.join("manufacturer"));
        d.serial = serial;
        d.details.insert("usbId".into(), json!(format!("{vid}:{pid}")));
        out.push(d);
    }
    out
}

pub fn collect() -> Result<Vec<Device>, String> {
    let root = Path::new("/");
    let hostname = read(Path::new("/proc/sys/kernel/hostname"));
    let mut out: Vec<Device> = computer(root, hostname).into_iter().collect();
    out.extend(usb(root));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sysfs() {
        let dir = std::env::temp_dir().join(format!("dt-agent-test-{}", std::process::id()));
        let dmi = dir.join("sys/class/dmi/id");
        fs::create_dir_all(&dmi).unwrap();
        fs::write(dmi.join("sys_vendor"), "LENOVO\n").unwrap();
        fs::write(dmi.join("product_name"), "20XW\n").unwrap();
        fs::write(dmi.join("chassis_type"), "10\n").unwrap();
        let bus = dir.join("sys/bus/usb/devices");
        for (port, vid, class, serial) in
            [("1-1", "046d", "00", Some("ABC123")), ("1-2", "05e3", "09", None), ("1-3", "1532", "00", None)]
        {
            fs::create_dir_all(bus.join(port)).unwrap();
            fs::write(bus.join(port).join("idVendor"), vid).unwrap();
            fs::write(bus.join(port).join("idProduct"), "c077").unwrap();
            fs::write(bus.join(port).join("bDeviceClass"), class).unwrap();
            if let Some(s) = serial {
                fs::write(bus.join(port).join("serial"), s).unwrap();
            }
        }
        let pc = computer(&dir, Some("dev-box".into())).unwrap();
        assert_eq!((pc.name.as_str(), pc.serial.as_deref(), pc.model.as_deref()), ("dev-box", None, Some("20XW")));
        assert_eq!(pc.details["formFactor"], "laptop");
        let found = usb(&dir);
        assert_eq!(found.len(), 2, "hubs are skipped");
        assert_eq!(found[0].serial.as_deref(), Some("ABC123"));
        assert_eq!(found[1].serial, None);
        assert_eq!(found[1].local_id, "usb:1532:c077:1-3");
        fs::remove_dir_all(dir).unwrap();
    }
}
