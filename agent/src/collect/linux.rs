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
    hardware(root, &mut d);
    Some(d)
}

/// CPU, memory, disks and operating system from /proc, /sys/block and /etc/os-release.
fn hardware(root: &Path, d: &mut Device) {
    if let Some(info) = read(&root.join("proc/cpuinfo")) {
        let field = |line: &str, key: &str| {
            line.split_once(':').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_string())
        };
        let name = info.lines().find_map(|l| field(l, "model name"));
        let threads = info.lines().filter(|l| field(l, "processor").is_some()).count();
        let mut cores: Vec<(String, String)> = info
            .split("\n\n")
            .filter_map(|block| {
                let get = |k| block.lines().find_map(|l| field(l, k));
                Some((get("physical id").unwrap_or_default(), get("core id")?))
            })
            .collect();
        cores.sort();
        cores.dedup();
        let mut c = serde_json::Map::new();
        if let Some(n) = name {
            c.insert("name".into(), json!(n));
        }
        if !cores.is_empty() {
            c.insert("cores".into(), json!(cores.len()));
        }
        if threads > 0 {
            c.insert("threads".into(), json!(threads));
        }
        if !c.is_empty() {
            d.details.insert("cpu".into(), serde_json::Value::Object(c));
        }
    }
    let kb = read(&root.join("proc/meminfo"))
        .and_then(|m| m.lines().find(|l| l.starts_with("MemTotal:"))?.split_whitespace().nth(1)?.parse::<f64>().ok());
    if let Some(kb) = kb {
        d.details.insert("memory".into(), json!({ "totalGb": (kb / 1_048_576.0 * 10.0).round() / 10.0 }));
    }
    // Whole disks only: loop, RAM and device-mapper devices are not hardware.
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(root.join("sys/block")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if ["loop", "ram", "dm-", "zram", "sr"].iter().any(|p| name.starts_with(p)) {
                continue;
            }
            total += read(&e.path().join("size")).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) * 512;
        }
    }
    if total > 0 {
        d.details.insert("storage".into(), json!({ "totalGb": total / 1_000_000_000 }));
    }
    let mut os = serde_json::Map::new();
    if let Some(release) = read(&root.join("etc/os-release")) {
        for (k, key) in [("name", "PRETTY_NAME"), ("version", "VERSION_ID")] {
            if let Some(v) = release.lines().find_map(|l| l.strip_prefix(&format!("{key}="))) {
                os.insert(k.into(), json!(v.trim_matches('"')));
            }
        }
    }
    if let Some(kernel) = read(&root.join("proc/sys/kernel/osrelease")) {
        os.insert("kernel".into(), json!(kernel));
    }
    if !os.is_empty() {
        os.insert("architecture".into(), json!(std::env::consts::ARCH));
        d.details.insert("os".into(), serde_json::Value::Object(os));
    }
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
        fs::create_dir_all(dir.join("proc/sys/kernel")).unwrap();
        fs::write(
            dir.join("proc/cpuinfo"),
            "processor\t: 0\nmodel name\t: AMD Ryzen 7 7840U\nphysical id\t: 0\ncore id\t: 0\n\nprocessor\t: 1\nmodel name\t: AMD Ryzen 7 7840U\nphysical id\t: 0\ncore id\t: 0\n\nprocessor\t: 2\nmodel name\t: AMD Ryzen 7 7840U\nphysical id\t: 0\ncore id\t: 1\n",
        )
        .unwrap();
        fs::write(dir.join("proc/meminfo"), "MemTotal:       32562148 kB\nMemFree: 1 kB\n").unwrap();
        fs::write(dir.join("proc/sys/kernel/osrelease"), "6.8.0-45-generic\n").unwrap();
        fs::create_dir_all(dir.join("etc")).unwrap();
        fs::write(
            dir.join("etc/os-release"),
            "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nVERSION_ID=\"24.04\"\n",
        )
        .unwrap();
        for (disk, sectors) in [("nvme0n1", "976773168"), ("loop0", "1000")] {
            fs::create_dir_all(dir.join("sys/block").join(disk)).unwrap();
            fs::write(dir.join("sys/block").join(disk).join("size"), sectors).unwrap();
        }
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
        assert_eq!(pc.details["cpu"], json!({ "name": "AMD Ryzen 7 7840U", "cores": 2, "threads": 3 }));
        assert_eq!(pc.details["memory"]["totalGb"], 31.1);
        assert_eq!(pc.details["storage"]["totalGb"], 500, "loop devices are not counted");
        assert_eq!(pc.details["os"]["name"], "Ubuntu 24.04.1 LTS");
        assert_eq!(pc.details["os"]["kernel"], "6.8.0-45-generic");
        let found = usb(&dir);
        assert_eq!(found.len(), 2, "hubs are skipped");
        assert_eq!(found[0].serial.as_deref(), Some("ABC123"));
        assert_eq!(found[1].serial, None);
        assert_eq!(found[1].local_id, "usb:1532:c077:1-3");
        fs::remove_dir_all(dir).unwrap();
    }
}
