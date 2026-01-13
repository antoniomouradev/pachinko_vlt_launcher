use super::HardwareInfo;
use anyhow::{Context, Result};
use std::fs;
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
    let uuid = fs::read_to_string("/etc/machine-id")
        .context("Falha ao ler /etc/machine-id")?;
    Ok(uuid.trim().to_string())
}

fn get_disk_serials() -> Result<Vec<String>> {
    let output = Command::new("lsblk")
        .args(&["-o", "UUID", "-n"])
        .output()
        .context("Falha ao executar lsblk")?;

    let output_str = String::from_utf8(output.stdout)
        .context("Falha ao decodificar saída")?;

    let uuids: Vec<String> = output_str
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
        .collect();

    Ok(uuids)
}

fn get_processor() -> Result<String> {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo")
        .context("Falha ao ler /proc/cpuinfo")?;

    for line in cpuinfo.lines() {
        if line.starts_with("model name") {
            if let Some(name) = line.split(':').nth(1) {
                return Ok(name.trim().to_string());
            }
        }
    }

    anyhow::bail!("Processador não encontrado")
}

fn get_mac_address() -> Result<String> {
    // Lê primeira interface não-loopback
    let interfaces = fs::read_dir("/sys/class/net")
        .context("Falha ao ler /sys/class/net")?;

    for entry in interfaces {
        let entry = entry.context("Falha ao ler entrada")?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Ignora loopback
        if name_str == "lo" {
            continue;
        }

        let addr_path = entry.path().join("address");
        if let Ok(mac) = fs::read_to_string(&addr_path) {
            let mac = mac.trim();
            if !mac.is_empty() && mac != "00:00:00:00:00:00" {
                return Ok(mac.to_string());
            }
        }
    }

    anyhow::bail!("MAC address não encontrado")
}
