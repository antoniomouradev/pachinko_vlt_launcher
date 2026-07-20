#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
pub mod audio;
pub mod video;
pub mod buttonhub;

use serde::Serialize;
use sha2::{Sha256, Digest};

#[derive(Debug, Clone, Serialize)]
pub struct HardwareInfo {
    pub mac_address: String,
    /// BIOS UUID (`dmidecode -t system`, campo UUID) — gravado na
    /// motherboard, sobrevive reinstall do SO (diferente do `/etc/machine-id`
    /// usado antes, que é gerado pelo SO e muda a cada reinstall).
    pub bios_uuid: String,
    /// Serial da motherboard (`dmidecode -t baseboard`).
    pub baseboard_serial: String,
    /// Processor ID real (`dmidecode -t processor`, campo ID) — não
    /// confundir com `processor` abaixo, que é só a string do modelo
    /// (igual em toda máquina do mesmo modelo, não serve de identidade).
    pub cpu_id: String,
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

/// Combina as fontes de hardware real (BIOS UUID + motherboard + CPU ID +
/// disco físico + MAC) num hash só. `hostname` fica de fora de propósito —
/// é editável por qualquer um, não é identidade de hardware.
pub fn compute_fingerprint(hw: &HardwareInfo) -> String {
    let raw = format!(
        "{}:{}:{}:{}:{}",
        hw.bios_uuid,
        hw.baseboard_serial,
        hw.cpu_id,
        hw.disk_serials.join(","),
        hw.mac_address,
    );
    let hash = Sha256::digest(raw.as_bytes());
    format!("sha256:{:x}", hash)
}
