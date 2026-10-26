use serde::Serialize;
use serde_json::{Map, Value};

/// One piece of hardware as this computer sees it. Only what the system reports is filled in:
/// a serial number is never guessed, and purchase or warranty details are never sent.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// The system's own stable id for the device on this computer (e.g. a PnP instance id).
    pub local_id: String,
    /// computer, monitor, dock, printer, storage, network, peripheral, phone or other.
    pub class: &'static str,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    pub details: Map<String, Value>,
}

impl Device {
    pub fn new(local_id: impl Into<String>, class: &'static str, name: impl Into<String>) -> Device {
        Device {
            local_id: local_id.into(),
            class,
            name: name.into(),
            manufacturer: None,
            model: None,
            serial: None,
            details: Map::new(),
        }
    }
}

/// A trimmed, non-empty string from a JSON value.
pub fn text(v: &Value) -> Option<String> {
    v.as_str().map(|s| s.trim().trim_matches('\0').trim().to_string()).filter(|s| !s.is_empty())
}

/// PowerShell's ConvertTo-Json gives an object for one result and an array for several.
pub fn list(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        Value::Null => vec![],
        other => vec![other.clone()],
    }
}
