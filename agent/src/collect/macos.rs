//! macOS: reads `system_profiler -json`, which ships with macOS.

use serde_json::{json, Value};

use crate::device::{list, text, Device};

#[cfg(target_os = "macos")]
pub fn collect() -> Result<Vec<Device>, String> {
    let output = std::process::Command::new("system_profiler")
        .args(["-json", "SPHardwareDataType", "SPDisplaysDataType", "SPUSBDataType"])
        .output()
        .map_err(|e| format!("could not run system_profiler: {e}"))?;
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| format!("unexpected output: {e}"))?;
    Ok(parse(&value))
}

#[cfg(not(target_os = "macos"))]
pub fn collect() -> Result<Vec<Device>, String> {
    Err("macOS only".into())
}

fn usb_tree(items: &Value, out: &mut Vec<Device>) {
    for item in list(items) {
        if let Some(name) = text(&item["_name"]) {
            let is_hub = name.to_lowercase().contains("hub");
            let serial = text(&item["serial_num"]);
            let location = text(&item["location_id"]).unwrap_or_default();
            if !is_hub && item.get("vendor_id").is_some() {
                let id = format!(
                    "usb:{}:{}:{}",
                    text(&item["vendor_id"]).unwrap_or_default(),
                    text(&item["product_id"]).unwrap_or_default(),
                    serial.clone().unwrap_or(location)
                );
                let mut d = Device::new(id, "peripheral", name);
                d.manufacturer = text(&item["manufacturer"]);
                d.serial = serial;
                out.push(d);
            }
        }
        usb_tree(&item["_items"], out);
    }
}

pub fn parse(v: &Value) -> Vec<Device> {
    let mut out = vec![];
    if let Some(hw) = list(&v["SPHardwareDataType"]).first() {
        let mut d = Device::new("computer", "computer", text(&hw["machine_name"]).unwrap_or_else(|| "Mac".into()));
        d.manufacturer = Some("Apple".into());
        d.model = text(&hw["machine_model"]).or_else(|| text(&hw["model_number"]));
        d.serial = text(&hw["serial_number"]);
        d.details.insert(
            "formFactor".into(),
            json!(if text(&hw["machine_name"]).unwrap_or_default().contains("Book") { "laptop" } else { "desktop" }),
        );
        out.push(d);
    }
    for gpu in list(&v["SPDisplaysDataType"]) {
        for m in list(&gpu["spdisplays_ndrvs"]) {
            let Some(name) = text(&m["_name"]) else { continue };
            if m.get("spdisplays_connection_type").and_then(Value::as_str) == Some("spdisplays_internal") {
                continue;
            }
            let serial = text(&m["_spdisplays_display-serial-number"]);
            let id = format!(
                "display:{}:{}",
                text(&m["_spdisplays_display-product-id"]).unwrap_or_default(),
                serial.clone().unwrap_or(name.clone())
            );
            let mut d = Device::new(id, "monitor", name.clone());
            d.model = Some(name);
            d.serial = serial;
            out.push(d);
        }
    }
    usb_tree(&v["SPUSBDataType"], &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_system_profiler() {
        let sample = json!({
            "SPHardwareDataType": [{ "machine_name": "MacBook Pro", "machine_model": "Mac15,3", "serial_number": "C02XYZ123" }],
            "SPDisplaysDataType": [{ "spdisplays_ndrvs": [
                { "_name": "Color LCD", "spdisplays_connection_type": "spdisplays_internal" },
                { "_name": "DELL U2723QE", "_spdisplays_display-serial-number": "abc1", "_spdisplays_display-product-id": "41a3" }
            ]}],
            "SPUSBDataType": [{ "_name": "USB31Bus", "_items": [
                { "_name": "USB2.1 Hub", "vendor_id": "0x05e3", "_items": [
                    { "_name": "Magic Keyboard", "vendor_id": "0x05ac", "product_id": "0x029c", "serial_num": "F0T123", "manufacturer": "Apple Inc." }
                ]}
            ]}]
        });
        let devices = parse(&sample);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].serial.as_deref(), Some("C02XYZ123"));
        assert_eq!(devices[0].details["formFactor"], "laptop");
        assert_eq!(devices[1].name, "DELL U2723QE");
        assert_eq!(devices[2].serial.as_deref(), Some("F0T123"));
    }
}
