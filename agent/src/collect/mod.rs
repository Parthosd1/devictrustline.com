//! Hardware discovery. Each platform reads what its own system tools report.

// Every parser is built on every platform so its tests run anywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod linux;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod macos;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub mod windows;

use crate::device::Device;

pub fn collect() -> Result<Vec<Device>, String> {
    #[cfg(target_os = "windows")]
    return windows::collect();
    #[cfg(target_os = "linux")]
    return linux::collect();
    #[cfg(target_os = "macos")]
    return macos::collect();
    #[allow(unreachable_code)]
    Err("This platform is not supported yet".into())
}

pub fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}
