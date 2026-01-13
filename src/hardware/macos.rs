use super::HardwareInfo;
use anyhow::{Context, Result};
use std::process::Command;

pub fn collect() -> Result<HardwareInfo> {
    let hostname = hostname::get()
        .context("Falha ao obter hostname")?
        .to_string_lossy()
        .to_string();

    let mac = get_mac_address()
        .context("Falha ao obter MAC address")?;

    let uuid = get_machine_uuid()
        .context("Falha ao obter Machine UUID")?;

    let disk_serials = get_disk_serials()
        .unwrap_or_else(|_| vec![]);

    let processor = get_processor()
        .unwrap_or_else(|_| "Unknown".to_string());

    Ok(HardwareInfo {
        mac_address: mac,
        uuid,
        disk_serials,
        processor,
        hostname,
        serial_number: None,
    })
}

fn get_machine_uuid() -> Result<String> {
    let output = Command::new("ioreg")
        .args(&["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .context("Falha ao executar ioreg")?;

    let output_str = String::from_utf8(output.stdout)
        .context("Falha ao decodificar saída do ioreg")?;

    for line in output_str.lines() {
        if line.contains("IOPlatformUUID") {
            let parts: Vec<&str> = line.split('"').collect();
            if parts.len() >= 2 {
                for part in parts.iter().rev() {
                    let part = part.trim();
                    if part.contains('-') && part.len() >= 36 {
                        return Ok(part.to_string());
                    }
                }
            }
        }
    }

    anyhow::bail!("UUID não encontrado na saída do ioreg")
}

fn get_disk_serials() -> Result<Vec<String>> {
    let output = Command::new("diskutil")
        .args(&["info", "-all"])
        .output()
        .context("Falha ao executar diskutil")?;

    let output_str = String::from_utf8(output.stdout)
        .context("Falha ao decodificar saída")?;

    let mut uuids = Vec::new();
    for line in output_str.lines() {
        if line.contains("Volume UUID:") {
            if let Some(uuid) = line.split(':').nth(1) {
                let uuid = uuid.trim();
                if !uuid.is_empty() {
                    uuids.push(uuid.to_string());
                }
            }
        }
    }

    Ok(uuids)
}

fn get_processor() -> Result<String> {
    let output = Command::new("sysctl")
        .args(&["-n", "machdep.cpu.brand_string"])
        .output()
        .context("Falha ao executar sysctl")?;

    let processor = String::from_utf8(output.stdout)
        .context("Falha ao decodificar saída")?
        .trim()
        .to_string();

    if processor.is_empty() {
        anyhow::bail!("Processador não encontrado")
    }

    Ok(processor)
}

fn get_mac_address() -> Result<String> {
    // macOS: usa ifconfig para obter MAC address
    let output = Command::new("ifconfig")
        .output()
        .context("Falha ao executar ifconfig")?;

    let output_str = String::from_utf8(output.stdout)
        .context("Falha ao decodificar saída")?;

    for line in output_str.lines() {
        if line.contains("ether") {
            // Formato: "ether XX:XX:XX:XX:XX:XX"
            let parts: Vec<&str> = line.split_whitespace().collect();
            for (i, part) in parts.iter().enumerate() {
                if part == &"ether" && i + 1 < parts.len() {
                    let mac = parts[i + 1].trim();
                    // MAC address tem formato XX:XX:XX:XX:XX:XX (17 caracteres)
                    if mac.contains(':') && mac.len() == 17 && mac != "00:00:00:00:00:00" {
                        return Ok(mac.to_string());
                    }
                }
            }
        }
    }

    anyhow::bail!("MAC address não encontrado")
}
