use super::HardwareInfo;
use anyhow::{Context, Result};
use std::fs;
use std::process::Command;

/// Caminhos sysfs equivalentes ao que `dmidecode` lia via `/dev/mem` —
/// mesma fonte (BIOS/SMBIOS cacheado pelo kernel), sem depender de binário
/// externo. Continuam exigindo root (arquivos são `0400 root` no kernel).
const SYS_PRODUCT_UUID: &str = "/sys/class/dmi/id/product_uuid";
const SYS_BOARD_SERIAL: &str = "/sys/class/dmi/id/board_serial";

pub fn collect() -> Result<HardwareInfo> {
    let hostname = hostname::get()
        .context("Falha ao obter hostname")?
        .to_string_lossy()
        .to_string();

    let mac = get_mac_address()
        .context("Falha ao obter MAC address")?;

    let bios_uuid = get_bios_uuid()
        .context("Falha ao obter BIOS UUID via sysfs")?;

    let baseboard_serial = get_baseboard_serial()
        .context("Falha ao obter Serial Number da motherboard via sysfs")?;

    let cpu_id = get_cpu_id()
        .context("Falha ao obter Processor ID via CPUID")?;

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

/// Filtra placeholders comuns de BIOS/motherboard genérica ou virtualizada
/// ("Not Specified", "To Be Filled By O.E.M.", vazio, UUID zerado) — não
/// são valor real de identidade.
fn validate_dmi_value(raw: &str) -> Option<String> {
    let value = raw.trim();
    let hex_digits: String = value.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let all_zero = !hex_digits.is_empty() && hex_digits.chars().all(|c| c == '0');
    if value.is_empty()
        || value.eq_ignore_ascii_case("Not Specified")
        || value.eq_ignore_ascii_case("To Be Filled By O.E.M.")
        || value.eq_ignore_ascii_case("None")
        || all_zero
    {
        None
    } else {
        Some(value.to_string())
    }
}

/// Lê direto do sysfs (mesma fonte que `dmidecode` usava por baixo) —
/// sem shell-out, sem depender de pacote externo instalado. Continua
/// exigindo root: o kernel restringe esses arquivos a `0400 root`.
fn read_sys_dmi(path: &str) -> Result<String> {
    fs::read_to_string(path)
        .with_context(|| format!("Falha ao ler {} (precisa de root)", path))
}

fn get_bios_uuid() -> Result<String> {
    let raw = read_sys_dmi(SYS_PRODUCT_UUID)?;
    validate_dmi_value(&raw)
        .with_context(|| format!("UUID vazio/placeholder em {}", SYS_PRODUCT_UUID))
}

fn get_baseboard_serial() -> Result<String> {
    let raw = read_sys_dmi(SYS_BOARD_SERIAL)?;
    validate_dmi_value(&raw)
        .with_context(|| format!("Serial Number vazio/placeholder em {}", SYS_BOARD_SERIAL))
}

/// O campo "Processor ID" do SMBIOS é, por spec, a concatenação crua de
/// EAX+EDX da instrução `CPUID(eax=1)` cacheada pela BIOS — dá pra pedir
/// esse mesmo valor direto, sem dmidecode e **sem precisar de root**
/// (CPUID é instrução não-privilegiada).
#[cfg(target_arch = "x86_64")]
fn get_cpu_id() -> Result<String> {
    let result = unsafe { std::arch::x86_64::__cpuid(1) };
    let bytes: Vec<u8> = result
        .eax
        .to_le_bytes()
        .into_iter()
        .chain(result.edx.to_le_bytes())
        .collect();
    Ok(bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" "))
}

#[cfg(not(target_arch = "x86_64"))]
fn get_cpu_id() -> Result<String> {
    anyhow::bail!("Processor ID via CPUID só suportado em x86_64")
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

/// Achado real: máquina com 2+ interfaces reais (ex: `eno1` ethernet +
/// `wlp1s0` wifi) tinha fingerprint mudando **entre boots** — `read_dir`
/// não garante ordem estável, então "a primeira não-loopback" trocava de
/// interface aleatoriamente a cada boot, mudando o MAC escolhido e por
/// tabela o fingerprint inteiro (servidor rejeitava com "hardware
/// fingerprint mismatch", 401, e o launcher apagava o próprio pareamento
/// por design — não era bug de disco/filesystem, era isso). Fix: ordena
/// por nome antes de escolher, sempre a mesma interface na mesma máquina.
fn get_mac_address() -> Result<String> {
    let mut names: Vec<String> = fs::read_dir("/sys/class/net")
        .context("Falha ao ler /sys/class/net")?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name != "lo")
        .collect();
    names.sort();

    for name in names {
        let addr_path = format!("/sys/class/net/{}/address", name);
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
mod dmi_validation_tests {
    use super::*;

    #[test]
    fn accepts_real_uuid() {
        let uuid = validate_dmi_value("4c4c4544-0043-3610-8058-b9c04f503332\n");
        assert_eq!(uuid.as_deref(), Some("4c4c4544-0043-3610-8058-b9c04f503332"));
    }

    #[test]
    fn accepts_real_serial() {
        let serial = validate_dmi_value(".XYZ456.\n");
        assert_eq!(serial.as_deref(), Some(".XYZ456."));
    }

    #[test]
    fn rejects_placeholder_value() {
        assert_eq!(validate_dmi_value("Not Specified\n"), None);
        assert_eq!(validate_dmi_value("To Be Filled By O.E.M.\n"), None);
        assert_eq!(validate_dmi_value("None\n"), None);
    }

    #[test]
    fn rejects_all_zero_uuid_from_vm() {
        // UUID zerado é comum em VM/placa genérica sem SMBIOS de verdade preenchido —
        // não é identidade real, tratar como ausente
        let uuid = validate_dmi_value("00000000-0000-0000-0000-000000000000\n");
        assert_eq!(uuid, None);
    }

    #[test]
    fn rejects_empty_value() {
        assert_eq!(validate_dmi_value("\n"), None);
        assert_eq!(validate_dmi_value(""), None);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn cpu_id_matches_expected_format() {
        // Formato: 8 bytes hex maiúsculo separados por espaço (EAX+EDX de CPUID(1))
        let id = get_cpu_id().expect("CPUID(1) deve funcionar em qualquer x86_64");
        let parts: Vec<&str> = id.split(' ').collect();
        assert_eq!(parts.len(), 8);
        for part in parts {
            assert_eq!(part.len(), 2);
            assert!(part.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase()));
        }
    }
}
