use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct PairResponse {
    pub status: String,
    pub machine_code: String,
    pub token: String,
    #[serde(default)]
    pub rgs_url: String,
    #[serde(default)]
    pub rgs_port: serde_json::Value,
    #[serde(default)]
    pub coin_list: Vec<u32>,
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub status: String,
    pub token: String,
    #[serde(default)]
    pub rgs_url: String,
    #[serde(default)]
    pub rgs_port: serde_json::Value,
    #[serde(default)]
    pub coin_list: Vec<u32>,
}

#[derive(Debug, Serialize)]
struct PairRequest<'a> {
    pairing_code: &'a str,
    hardware_fingerprint: &'a str,
}

#[derive(Debug, Serialize)]
struct LauncherRequest<'a> {
    machine_code: &'a str,
    hardware_fingerprint: &'a str,
}

#[derive(Debug, Serialize)]
struct HeartbeatRequest<'a> {
    machine_code: &'a str,
}

fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Falha ao criar cliente HTTP")
}

pub async fn pair_machine(cs_url: &str, pairing_code: &str, fingerprint: &str) -> Result<PairResponse> {
    let client = build_client()?;
    let url = format!("{}/machine/pair", cs_url);

    let resp = client
        .post(&url)
        .json(&PairRequest { pairing_code, hardware_fingerprint: fingerprint })
        .send()
        .await
        .with_context(|| format!("Falha ao conectar ao CS em {}", url))?;

    let status = resp.status();
    if status.is_success() {
        resp.json::<PairResponse>().await.context("Falha ao decodificar resposta de pair")
    } else {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("pair_machine HTTP {}: {}", status, body)
    }
}

pub async fn get_token(cs_url: &str, machine_code: &str, fingerprint: &str) -> Result<TokenResponse> {
    let client = build_client()?;
    let url = format!("{}/launcher", cs_url);

    let resp = client
        .post(&url)
        .json(&LauncherRequest { machine_code, hardware_fingerprint: fingerprint })
        .send()
        .await
        .with_context(|| format!("Falha ao conectar ao CS em {}", url))?;

    let status = resp.status();
    if status.is_success() {
        resp.json::<TokenResponse>().await.context("Falha ao decodificar resposta de token")
    } else {
        let code = status.as_u16();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("get_token HTTP {}: {}", code, body)
    }
}

/// Endpoint público só pra checagem de conectividade (não é o CS) — mesma
/// ideia do "captive portal check" que Android/ChromeOS usam.
const INTERNET_CHECK_URL: &str = "https://1.1.1.1";

/// Testa se a máquina tem internet, sem depender do CS estar no ar — é o que
/// o técnico em campo quer saber (rede física/wifi funcionando), não se o
/// backend específico responde.
pub async fn check_connection() -> Result<std::time::Duration> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("Falha ao criar cliente HTTP")?;

    let start = std::time::Instant::now();
    client
        .get(INTERNET_CHECK_URL)
        .send()
        .await
        .context("Falha ao conectar à internet")?;

    Ok(start.elapsed())
}

pub async fn send_heartbeat(cs_url: &str, machine_code: &str) -> Result<()> {
    let client = build_client()?;
    let url = format!("{}/machine/heartbeat", cs_url);

    let resp = client
        .post(&url)
        .json(&HeartbeatRequest { machine_code })
        .send()
        .await
        .with_context(|| format!("Falha ao enviar heartbeat para {}", url))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let code = resp.status().as_u16();
        anyhow::bail!("heartbeat HTTP {}", code)
    }
}
