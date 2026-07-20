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

    let bios_uuid = get_bios_uuid()
        .context("Falha ao obter BIOS UUID via dmidecode")?;

    let baseboard_serial = get_baseboard_serial()
        .context("Falha ao obter Serial Number da motherboard via dmidecode")?;

    let cpu_id = get_cpu_id()
        .context("Falha ao obter Processor ID via dmidecode")?;

    let disk_serials = get_disk_serials()
        .unwrap_or_else(|_| vec![]);

    let processor = get_processor()
        .unwrap_or_else(|_| "Unknown".to_string());

    Ok(HardwareInfo {
        mac_address: mac,
        bios_uuid,
        baseboard_serial,
        cpu_id,
        disk_serials,
        processor,
        hostname,
        serial_number: None,
    })
}

/// Roda `dmidecode -t <dmi_type>` e devolve a saída crua (stdout).
/// Exige root — se o processo não tiver privilégio suficiente, dmidecode
/// sai com erro/saída vazia e isso vira `Err` aqui.
fn run_dmidecode(dmi_type: &str) -> Result<String> {
    let output = Command::new("dmidecode")
        .args(&["-t", dmi_type])
        .output()
        .context("Falha ao executar dmidecode (precisa de root)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("dmidecode -t {} falhou: {}", dmi_type, stderr);
    }

    String::from_utf8(output.stdout).context("Falha ao decodificar saída do dmidecode")
}

/// Parser puro (sem I/O) do formato `dmidecode`: linhas `\tCampo: valor`.
/// Filtra placeholders comuns de BIOS/motherboard genérica ou virtualizada
/// ("Not Specified", "To Be Filled By O.E.M.", vazio) — não são valor real.
fn parse_dmi_field(output: &str, field: &str) -> Option<String> {
    let prefix = format!("{}:", field);
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix(&prefix) {
            let value = value.trim();
            let hex_digits: String = value.chars().filter(|c| c.is_ascii_hexdigit()).collect();
            let all_zero = !hex_digits.is_empty() && hex_digits.chars().all(|c| c == '0');
            if value.is_empty()
                || value.eq_ignore_ascii_case("Not Specified")
                || value.eq_ignore_ascii_case("To Be Filled By O.E.M.")
                || value.eq_ignore_ascii_case("None")
                || all_zero
            {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

fn get_bios_uuid() -> Result<String> {
    let output = run_dmidecode("system")?;
    parse_dmi_field(&output, "UUID").context("Campo UUID não encontrado/vazio em dmidecode -t system")
}

fn get_baseboard_serial() -> Result<String> {
    let output = run_dmidecode("baseboard")?;
    parse_dmi_field(&output, "Serial Number")
        .context("Campo Serial Number não encontrado/vazio em dmidecode -t baseboard")
}

fn get_cpu_id() -> Result<String> {
    let output = run_dmidecode("processor")?;
    parse_dmi_field(&output, "ID").context("Campo ID não encontrado/vazio em dmidecode -t processor")
}

/// Serial físico do disco (não muda ao reformatar, diferente do UUID de
/// filesystem que `lsblk -o UUID` dava antes). `-d` restringe a discos
/// inteiros, sem listar cada partição separada.
fn get_disk_serials() -> Result<Vec<String>> {
    let output = Command::new("lsblk")
        .args(&["-o", "SERIAL", "-n", "-d"])
        .output()
        .context("Falha ao executar lsblk")?;

    let output_str = String::from_utf8(output.stdout)
        .context("Falha ao decodificar saída")?;

    let serials: Vec<String> = output_str
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

    Ok(serials)
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

#[cfg(test)]
mod dmi_parsing_tests {
    use super::*;

    const SYSTEM_INFO_SAMPLE: &str = "\
# dmidecode 3.3
Getting SMBIOS data from sysfs.
SMBIOS 2.8 present.

Handle 0x0001, DMI type 1, 27 bytes
System Information
\tManufacturer: Dell Inc.
\tProduct Name: OptiPlex 7090
\tVersion: Not Specified
\tSerial Number: ABCD123
\tUUID: 4c4c4544-0043-3610-8058-b9c04f503332
\tWake-up Type: Power Switch
\tSKU Number:
\tFamily:
";

    const BASEBOARD_SAMPLE: &str = "\
Handle 0x0002, DMI type 2, 15 bytes
Base Board Information
\tManufacturer: Dell Inc.
\tProduct Name: 0ABC123
\tVersion: A01
\tSerial Number: .XYZ456.
\tAsset Tag: Not Specified
\tFeatures:
\t\tBoard is a hosting board
\tLocation In Chassis: Not Specified
";

    const PROCESSOR_SAMPLE: &str = "\
Handle 0x0004, DMI type 4, 42 bytes
Processor Information
\tSocket Designation: CPU1
\tType: Central Processor
\tFamily: Core i7
\tManufacturer: Intel(R) Corporation
\tID: A9 06 08 00 FF FB EB BF
\tVersion: Intel(R) Core(TM) i7-9700 CPU @ 3.00GHz
\tVoltage: 1.2 V
";

    const VM_SYSTEM_INFO_SAMPLE: &str = "\
Handle 0x0001, DMI type 1, 27 bytes
System Information
\tManufacturer: QEMU
\tProduct Name: Standard PC
\tVersion: pc-i440fx-2.1
\tSerial Number: Not Specified
\tUUID: 00000000-0000-0000-0000-000000000000
\tWake-up Type: Power Switch
";

    #[test]
    fn parses_bios_uuid_from_system_info() {
        let uuid = parse_dmi_field(SYSTEM_INFO_SAMPLE, "UUID");
        assert_eq!(uuid.as_deref(), Some("4c4c4544-0043-3610-8058-b9c04f503332"));
    }

    #[test]
    fn parses_baseboard_serial() {
        let serial = parse_dmi_field(BASEBOARD_SAMPLE, "Serial Number");
        assert_eq!(serial.as_deref(), Some(".XYZ456."));
    }

    #[test]
    fn parses_cpu_id() {
        let id = parse_dmi_field(PROCESSOR_SAMPLE, "ID");
        assert_eq!(id.as_deref(), Some("A9 06 08 00 FF FB EB BF"));
    }

    #[test]
    fn rejects_placeholder_serial_number() {
        // Version na system info é "Not Specified" — deve virar None, não string literal
        let version = parse_dmi_field(SYSTEM_INFO_SAMPLE, "Version");
        assert_eq!(version, None);
    }

    #[test]
    fn rejects_all_zero_uuid_from_vm() {
        // UUID zerado é comum em VM/placa genérica sem SMBIOS de verdade preenchido —
        // não é identidade real, tratar como ausente
        let uuid = parse_dmi_field(VM_SYSTEM_INFO_SAMPLE, "UUID");
        assert_eq!(uuid, None);
    }

    #[test]
    fn missing_field_returns_none() {
        let missing = parse_dmi_field(SYSTEM_INFO_SAMPLE, "Asset Tag");
        assert_eq!(missing, None);
    }

    #[test]
    fn empty_field_value_returns_none() {
        let sku = parse_dmi_field(SYSTEM_INFO_SAMPLE, "SKU Number");
        assert_eq!(sku, None);
    }
}
