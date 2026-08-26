use anyhow::{Context, Result};
use std::fs;
use std::process::Command;

const SERVICE_NAME: &str = "pachinko-launcher";
const SERVICE_FILE: &str = "/etc/systemd/system/pachinko-launcher.service";
const BINARY_NAME: &str = "pachinko_vlt_launcher";

pub fn install_service() -> Result<()> {
    if unsafe { libc::getuid() } != 0 {
        anyhow::bail!("Instalação de serviço requer privilégios de root (sudo)");
    }

    let service_content = format!(
        r#"[Unit]
Description=Pachinko VLT Launcher - Registro e Autenticação de Máquinas
After=network.target buttonhub.service
Wants=network-online.target
Wants=buttonhub.service

[Service]
Type=simple
ExecStart=/usr/local/bin/{}
Restart=on-failure
RestartSec=5
StandardInput=tty
StandardOutput=tty
StandardError=journal
TTYPath=/dev/tty1
TTYReset=yes
TTYVTDisallocate=no
Environment="CS_URL={}"
Environment="GAME_REGISTRY_URL={}"
Environment="DISPLAY=:0.0"
Environment="XAUTHORITY=/root/.Xauthority"
Environment="RUST_LOG=info"

[Install]
WantedBy=multi-user.target
"#,
        BINARY_NAME,
        crate::env_config::default_cs_url(),
        crate::env_config::default_game_registry_url(),
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
