use crate::hardware::HardwareInfo;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct RegisterRequest {
    hardware_info: HardwareInfo,
}

#[derive(Debug, Deserialize)]
pub struct RegisterResponse {
    pub status: String,
    pub machine_id: u32,
    pub hardware_fingerprint: String,
    #[serde(default)]
    pub registration_pin: Option<String>,
    pub requires_approval: bool,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub signing_secret: Option<String>,
}

pub async fn register_machine(
    rgs_url: &str,
    hardware_info: &HardwareInfo,
) -> Result<RegisterResponse> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Falha ao criar cliente HTTP")?;

    let url = format!("{}/api/vlt/register_machine", rgs_url);

    let request = RegisterRequest {
        hardware_info: hardware_info.clone(),
    };

    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await
        .with_context(|| {
            format!(
                "Falha ao conectar ao servidor RGS em {}. Verifique se:\n  - O servidor RGS está rodando\n  - A URL está correta (use RGS_URL=http://host:port)\n  - A porta está acessível",
                url
            )
        })?;

    if response.status().is_success() {
        let result: RegisterResponse = response
            .json()
            .await
            .context("Falha ao decodificar resposta JSON")?;
        Ok(result)
    } else {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "Erro ao registrar máquina: HTTP {} - {}",
            status,
            error_text
        )
    }
}
