//! Windows: one PowerShell call reads CIM/WMI classes (built into Windows 10 and later) and
//! returns JSON, which is turned into devices here.

use serde_json::{json, Value};

use crate::device::{list, text, Device};

/// Read by `collect`; also kept as text so it can be reviewed and run by hand.
pub const SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$out = [ordered]@{
  system    = Get-CimInstance Win32_ComputerSystem | Select-Object Name, Manufacturer, Model, SystemFamily
  bios      = Get-CimInstance Win32_BIOS | Select-Object SerialNumber
  enclosure = Get-CimInstance Win32_SystemEnclosure | Select-Object ChassisTypes
  monitors  = Get-CimInstance -Namespace root\wmi WmiMonitorID | Select-Object InstanceName, ManufacturerName, UserFriendlyName, SerialNumberID, ProductCodeID
  disks     = Get-CimInstance Win32_DiskDrive | Select-Object Model, SerialNumber, InterfaceType, MediaType, PNPDeviceID, Size
  printers  = Get-CimInstance Win32_Printer | Select-Object Name, DriverName, PortName, Local, Network, PNPDeviceID
  pnp       = Get-CimInstance Win32_PnPEntity -Filter "Present = TRUE" | Where-Object { $_.PNPClass -in @('Mouse','Keyboard','Camera','Image','Biometric','SmartCardReader','WPD','Bluetooth','USB') } | Select-Object Name, Manufacturer, PNPClass, PNPDeviceID, Service
  adapters  = Get-CimInstance Win32_NetworkAdapter -Filter "PhysicalAdapter = TRUE" | Select-Object Name, Manufacturer, MACAddress, PNPDeviceID, AdapterType
}
$out | ConvertTo-Json -Depth 4 -Compress
"#;

#[cfg(target_os = "windows")]
pub fn collect() -> Result<Vec<Device>, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("could not run PowerShell: {e}"))?;
    if !output.status.success() {
        return Err(format!("PowerShell failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    let value: Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("unexpected PowerShell output: {e}"))?;
    Ok(parse(&value))
}

#[cfg(not(target_os = "windows"))]
pub fn collect() -> Result<Vec<Device>, String> {
    Err("Windows only".into())
}

/// WmiMonitorID stores strings as arrays of UTF-16 code units, zero-padded.
fn wmi_string(v: &Value) -> Option<String> {
    let units: Vec<u16> =
        v.as_array()?.iter().filter_map(|n| n.as_u64()).map(|n| n as u16).take_while(|n| *n != 0).collect();
    let s = String::from_utf16_lossy(&units).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Windows writes a device's own serial as the last part of a USB instance id. Ids it made up
/// itself contain '&', so those are not serials.
pub fn usb_serial(pnp_id: &str) -> Option<String> {
    if !pnp_id.to_uppercase().starts_with("USB\\") {
        return None;
    }
    let last = pnp_id.rsplit('\\').next()?;
    (last.len() >= 4 && !last.contains('&')).then(|| last.to_string())
}

fn chassis_class(types: &Value) -> &'static str {
    // https://learn.microsoft.com/windows/win32/cimwin32prov/win32-systemenclosure
    let laptop = [8, 9, 10, 14, 30, 31, 32];
    match list(types).iter().filter_map(Value::as_u64).next() {
        Some(t) if laptop.contains(&t) => "laptop",
        Some(_) => "desktop",
        None => "unknown",
    }
}

const VIRTUAL_PRINTERS: &[&str] =
    &["Microsoft Print to PDF", "Microsoft XPS Document Writer", "OneNote", "Fax", "Send To OneNote"];

pub fn parse(v: &Value) -> Vec<Device> {
    let mut out = vec![];
    if let Some(system) = list(&v["system"]).first() {
        let name = text(&system["Name"]).unwrap_or_else(|| "This computer".into());
        let mut d = Device::new("computer", "computer", name);
        d.manufacturer = text(&system["Manufacturer"]);
        d.model = text(&system["Model"]).or_else(|| text(&system["SystemFamily"]));
        d.serial = list(&v["bios"]).first().and_then(|b| text(&b["SerialNumber"]));
        d.details.insert(
            "formFactor".into(),
            json!(chassis_class(&list(&v["enclosure"]).first().cloned().unwrap_or_default()["ChassisTypes"])),
        );
        out.push(d);
    }
    for m in list(&v["monitors"]) {
        let Some(id) = text(&m["InstanceName"]) else { continue };
        let name = wmi_string(&m["UserFriendlyName"]).unwrap_or_else(|| "Monitor".into());
        let mut d = Device::new(id, "monitor", name.clone());
        d.manufacturer = wmi_string(&m["ManufacturerName"]);
        d.model = Some(name);
        d.serial = wmi_string(&m["SerialNumberID"]);
        if let Some(code) = wmi_string(&m["ProductCodeID"]) {
            d.details.insert("productCode".into(), json!(code));
        }
        out.push(d);
    }
    for disk in list(&v["disks"]) {
        let Some(id) = text(&disk["PNPDeviceID"]) else { continue };
        let model = text(&disk["Model"]).unwrap_or_else(|| "Disk".into());
        let mut d = Device::new(id, "storage", model.clone());
        d.model = Some(model);
        d.serial = text(&disk["SerialNumber"]);
        for (k, key) in [("interface", "InterfaceType"), ("media", "MediaType")] {
            if let Some(t) = text(&disk[key]) {
                d.details.insert(k.into(), json!(t));
            }
        }
        if let Some(size) = disk["Size"].as_u64() {
            d.details.insert("sizeGb".into(), json!(size / 1_000_000_000));
        }
        out.push(d);
    }
    for p in list(&v["printers"]) {
        let Some(name) = text(&p["Name"]) else { continue };
        if VIRTUAL_PRINTERS.iter().any(|v| name.starts_with(v)) {
            continue;
        }
        let id = text(&p["PNPDeviceID"]).unwrap_or_else(|| format!("printer:{name}"));
        let mut d = Device::new(id, "printer", name);
        d.model = text(&p["DriverName"]);
        if let Some(port) = text(&p["PortName"]) {
            d.details.insert("port".into(), json!(port));
        }
        d.details.insert("network".into(), json!(p["Network"] == true));
        out.push(d);
    }
    for e in list(&v["pnp"]) {
        let (Some(id), Some(name)) = (text(&e["PNPDeviceID"]), text(&e["Name"])) else { continue };
        let class = text(&e["PNPClass"]).unwrap_or_default();
        // Hubs and controllers are part of the computer, not separate equipment.
        if class == "USB" && (name.contains("Hub") || name.contains("Controller") || name.contains("Composite")) {
            continue;
        }
        let mut d = Device::new(id.clone(), if class == "WPD" { "phone" } else { "peripheral" }, name);
        d.manufacturer = text(&e["Manufacturer"]).filter(|m| !m.starts_with('(') && m != "Microsoft");
        d.serial = usb_serial(&id);
        d.details.insert("kind".into(), json!(class));
        out.push(d);
    }
    for a in list(&v["adapters"]) {
        let (Some(id), Some(name)) = (text(&a["PNPDeviceID"]), text(&a["Name"])) else { continue };
        // Built-in adapters belong to the computer; only USB adapters and docks are separate.
        if !id.to_uppercase().starts_with("USB\\") {
            continue;
        }
        let dock = name.to_lowercase().contains("dock");
        let mut d = Device::new(id.clone(), if dock { "dock" } else { "network" }, name);
        d.manufacturer = text(&a["Manufacturer"]);
        d.serial = usb_serial(&id);
        if let Some(mac) = text(&a["MACAddress"]) {
            d.details.insert("mac".into(), json!(mac));
        }
        out.push(d);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Value {
        let mut v: Vec<u16> = s.encode_utf16().collect();
        v.resize(16, 0);
        json!(v)
    }

    #[test]
    fn parses_a_typical_office_pc() {
        let sample = json!({
            "system": { "Name": "FRONTDESK-01", "Manufacturer": "Dell Inc.", "Model": "OptiPlex 7010" },
            "bios": { "SerialNumber": "7XK2Q93" },
            "enclosure": { "ChassisTypes": [3] },
            "monitors": [
                { "InstanceName": "DISPLAY\\DELA0F5\\5&1_0", "ManufacturerName": utf16("DEL"), "UserFriendlyName": utf16("DELL P2422H"), "SerialNumberID": utf16("CN0ABC123") },
                { "InstanceName": "DISPLAY\\GSM5B7F\\5&2_0", "ManufacturerName": utf16("GSM"), "UserFriendlyName": utf16("LG FHD"), "SerialNumberID": utf16("0") }
            ],
            "disks": { "Model": "Samsung SSD 980", "SerialNumber": " S64DNX0R123456 ", "InterfaceType": "SCSI", "PNPDeviceID": "SCSI\\DISK&VEN_NVME\\5&1", "Size": 500107862016u64 },
            "printers": [{ "Name": "Microsoft Print to PDF" }, { "Name": "HP LaserJet M404", "DriverName": "HP Universal", "PortName": "IP_10.0.0.5", "Network": true }],
            "pnp": [
                { "Name": "USB Input Device", "PNPClass": "Mouse", "PNPDeviceID": "HID\\VID_046D&PID_C077\\7&2B4A&0&0000", "Manufacturer": "(Standard system devices)" },
                { "Name": "Logitech BRIO", "PNPClass": "Camera", "PNPDeviceID": "USB\\VID_046D&PID_085E\\2F5A3B1C", "Manufacturer": "Logitech" },
                { "Name": "Generic USB Hub", "PNPClass": "USB", "PNPDeviceID": "USB\\VID_05E3&PID_0610\\5&1" }
            ],
            "adapters": [
                { "Name": "Intel Ethernet", "PNPDeviceID": "PCI\\VEN_8086", "MACAddress": "AA" },
                { "Name": "Dell USB-C Dock WD19", "PNPDeviceID": "USB\\VID_0BDA&PID_8153\\301000001", "MACAddress": "BB:CC" }
            ]
        });
        let devices = parse(&sample);
        let by = |id: &str| devices.iter().find(|d| d.local_id == id).unwrap();
        let pc = by("computer");
        assert_eq!((pc.name.as_str(), pc.serial.as_deref(), pc.class), ("FRONTDESK-01", Some("7XK2Q93"), "computer"));
        assert_eq!(pc.details["formFactor"], "desktop");
        let monitor = by("DISPLAY\\DELA0F5\\5&1_0");
        assert_eq!((monitor.manufacturer.as_deref(), monitor.serial.as_deref()), (Some("DEL"), Some("CN0ABC123")));
        // The server decides "0" identifies nothing; the agent reports what Windows said.
        assert_eq!(by("DISPLAY\\GSM5B7F\\5&2_0").serial.as_deref(), Some("0"));
        assert_eq!(by("SCSI\\DISK&VEN_NVME\\5&1").serial.as_deref(), Some("S64DNX0R123456"));
        assert!(devices
            .iter()
            .all(|d| d.name != "Microsoft Print to PDF" && d.name != "Generic USB Hub" && d.name != "Intel Ethernet"));
        let mouse = by("HID\\VID_046D&PID_C077\\7&2B4A&0&0000");
        assert_eq!((mouse.serial.as_deref(), mouse.manufacturer.as_deref()), (None, None), "nothing invented");
        assert_eq!(by("USB\\VID_046D&PID_085E\\2F5A3B1C").serial.as_deref(), Some("2F5A3B1C"));
        let dock = by("USB\\VID_0BDA&PID_8153\\301000001");
        assert_eq!(dock.class, "dock");
        assert_eq!(devices.len(), 8);
    }

    #[test]
    fn windows_generated_ids_are_not_serials() {
        assert_eq!(usb_serial("USB\\VID_1&PID_2\\6&3A1B&0&2"), None);
        assert_eq!(usb_serial("HID\\VID_1\\ABCDEF"), None);
        assert_eq!(usb_serial("USB\\VID_1&PID_2\\0001A2B3"), Some("0001A2B3".into()));
    }
}
