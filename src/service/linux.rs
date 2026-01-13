use anyhow::{Context, Result};
use std::fs;
use std::process::Command;

const SERVICE_NAME: &str = "pachinko-launcher";
const SERVICE_FILE: &str = "/etc/systemd/system/pachinko-launcher.service";

pub fn install_service() -> Result<()> {
    if unsafe { libc::getuid() } != 0 {
        anyhow::bail!("Instalação de serviço requer privilégios de root (sudo)");
    }

    let service_content = format!(
        r#"[Unit]
Description=Pachinko VLT Launcher - Registro e Autenticação de Máquinas
After=network.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/{}
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal
Environment="RGS_URL=http://localhost:43310"
Environment="RUST_LOG=info"

[Install]
WantedBy=multi-user.target
"#,
        SERVICE_NAME
    );

    fs::write(SERVICE_FILE, service_content)
        .context("Falha ao escrever arquivo de serviço systemd")?;

    Command::new("systemctl")
        .args(&["daemon-reload"])
        .status()
        .context("Falha ao recarregar systemd")?;

    Command::new("systemctl")
        .args(&["enable", SERVICE_NAME])
        .status()
        .context("Falha ao habilitar serviço")?;

    Command::new("systemctl")
        .args(&["start", SERVICE_NAME])
        .status()
        .context("Falha ao iniciar serviço")?;

    Ok(())
}

pub fn uninstall_service() -> Result<()> {
    if unsafe { libc::getuid() } != 0 {
        anyhow::bail!("Remoção de serviço requer privilégios de root (sudo)");
    }

    // Para serviço
    let _ = Command::new("systemctl")
        .args(&["stop", SERVICE_NAME])
        .status();

    // Desabilita serviço
    let _ = Command::new("systemctl")
        .args(&["disable", SERVICE_NAME])
        .status();

    // Remove arquivo
    if std::path::Path::new(SERVICE_FILE).exists() {
        fs::remove_file(SERVICE_FILE)
            .context("Falha ao remover arquivo de serviço")?;
    }

    Command::new("systemctl")
        .args(&["daemon-reload"])
        .status()
        .context("Falha ao recarregar systemd")?;

    Ok(())
}
