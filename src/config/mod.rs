use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce
};
use hkdf::Hkdf;
use sha2::Sha256;
use base64::{Engine as _, engine::general_purpose};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherConfig {
    pub machine_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_pin: Option<String>,
    pub registered_at: String,
    pub rgs_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_secret: Option<String>,
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

pub fn derive_encryption_key(
    hardware_fingerprint: &str,
    machine_id: u32,
) -> Result<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, hardware_fingerprint.as_bytes());
    let mut okm = [0u8; 32];
    let info = format!("pachinko_vlt_launcher:{}", machine_id);
    hk.expand(info.as_bytes(), &mut okm)
        .map_err(|e| anyhow::anyhow!("Falha ao derivar chave: {}", e))?;
    Ok(okm)
}

pub fn encrypt_secret(plaintext: &str, key: &[u8; 32]) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("Falha ao criar cipher: {}", e))?;
    
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("Falha ao criptografar: {}", e))?;
    
    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);
    
    Ok(general_purpose::STANDARD.encode(&combined))
}

pub fn decrypt_secret(ciphertext: &str, key: &[u8; 32]) -> Result<String> {
    let combined = general_purpose::STANDARD
        .decode(ciphertext)
        .map_err(|e| anyhow::anyhow!("Falha ao decodificar base64: {}", e))?;
    
    if combined.len() < 12 {
        anyhow::bail!("Ciphertext muito curto");
    }
    
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("Falha ao criar cipher: {}", e))?;
    
    let nonce = Nonce::from_slice(&combined[..12]);
    let ciphertext_bytes = &combined[12..];
    
    let plaintext = cipher
        .decrypt(nonce, ciphertext_bytes)
        .map_err(|e| anyhow::anyhow!("Falha ao descriptografar: {}", e))?;
    
    String::from_utf8(plaintext)
        .map_err(|e| anyhow::anyhow!("Falha ao converter para string: {}", e))
}
