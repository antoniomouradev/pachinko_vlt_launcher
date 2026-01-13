#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HardwareInfo {
    pub mac_address: String,
    pub uuid: String,
    pub disk_serials: Vec<String>,
    pub processor: String,
    pub hostname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
}

pub fn collect() -> anyhow::Result<HardwareInfo> {
    #[cfg(target_os = "linux")]
    return linux::collect();
    
    #[cfg(target_os = "macos")]
    return macos::collect();
    
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    anyhow::bail!("Sistema operacional não suportado. Use Linux ou macOS para testes.")
}
