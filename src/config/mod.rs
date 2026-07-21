use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

fn default_game_registry_url() -> String {
    "http://192.168.15.12:8090".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherSettings {
    pub cs_url: String,
    pub game_path: String,
    #[serde(default)]
    pub game_args: Vec<String>,
    /// Base do `game_registry_service` — de onde o jogo é baixado (RAM,
    /// nunca disco) a cada boot. `game_path` acima fica só pro fluxo antigo
    /// (binário já parado no disco), não usado quando o download funciona.
    #[serde(default = "default_game_registry_url")]
    pub game_registry_url: String,
}

fn default_machine_variant() -> String {
    "vlt".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherConfig {
    pub machine_code: String,
    pub hardware_fingerprint: String,
    pub cs_url: String,
    pub paired_at: String,
    /// `vlt`/`street`, decide qual build o `game_registry_service` serve.
    /// Default `vlt` pra configs salvos antes desse campo existir.
    #[serde(default = "default_machine_variant")]
    pub machine_variant: String,
}

pub fn get_config_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Não foi possível encontrar diretório de config"))?;

    let pachinko_dir = config_dir.join("pachinko");

    if !pachinko_dir.exists() {
        fs::create_dir_all(&pachinko_dir)
            .context("Falha ao criar diretório de configuração")?;
    }

    Ok(pachinko_dir.join("launcher_config.json"))
}

pub fn get_settings_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Não foi possível encontrar diretório de config"))?;
    let pachinko_dir = config_dir.join("pachinko");
    if !pachinko_dir.exists() {
        fs::create_dir_all(&pachinko_dir)
            .context("Falha ao criar diretório de configuração")?;
    }
    Ok(pachinko_dir.join("launcher_settings.json"))
}

pub fn load_settings(path: &Path) -> Result<LauncherSettings> {
    let content = fs::read_to_string(path)
        .context("Falha ao ler launcher_settings.json")?;
    serde_json::from_str(&content)
        .context("Falha ao parsear launcher_settings.json")
}

pub fn save_settings(path: &Path, settings: &LauncherSettings) -> Result<()> {
    let content = serde_json::to_string_pretty(settings)
        .context("Falha ao serializar settings")?;
    fs::write(path, content)
        .context("Falha ao escrever launcher_settings.json")
}

pub fn get_pairing_code_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Não foi possível encontrar diretório de config"))?;
    Ok(config_dir.join("pachinko").join("pairing_code"))
}

pub fn load_config(path: &Path) -> Result<LauncherConfig> {
    let content = fs::read_to_string(path)
        .context("Falha ao ler arquivo de configuração")?;

    let config: LauncherConfig = serde_json::from_str(&content)
        .context("Falha ao parsear JSON de configuração")?;

    Ok(config)
}

pub fn save_config(path: &Path, config: &LauncherConfig) -> Result<()> {
    let content = serde_json::to_string_pretty(config)
        .context("Falha ao serializar configuração")?;

    fs::write(path, content)
        .context("Falha ao escrever arquivo de configuração")?;

    Ok(())
}

pub fn delete_config(path: &Path) {
    let _ = fs::remove_file(path);
}
