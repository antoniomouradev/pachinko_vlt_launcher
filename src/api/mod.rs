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

#[derive(Debug, Deserialize)]
pub struct DeviceRegisterResponse {
    pub status: String,
    pub device_code: String,
    pub user_code: String,
    #[serde(default)]
    pub registration_status: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeviceTokenResponse {
    pub status: String,
    pub registration_status: String,
    #[serde(default)]
    pub machine_code: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub rgs_url: Option<String>,
    #[serde(default)]
    pub rgs_port: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct DeviceRegisterRequest<'a> {
    hardware_fingerprint: &'a str,
    /// Autodetectado por `hardware::video::detect_machine_type()` — não é
    /// escolhido pelo operador (ver `hardware/video.rs`).
    machine_type: &'a str,
}

#[derive(Debug, Serialize)]
struct DeviceTokenRequest<'a> {
    device_code: &'a str,
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

/// Fluxo antigo (pareamento OTP) — ver nota em `main.rs::wait_for_pairing`.
#[allow(dead_code)]
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

/// POST /device/register — autorregistro (device flow). Reenviar o mesmo
/// `fingerprint` depois de já registrado devolve o mesmo `user_code`, não
/// gera um novo (comportamento do servidor).
pub async fn register_device(cs_url: &str, fingerprint: &str, machine_type: &str) -> Result<DeviceRegisterResponse> {
    let client = build_client()?;
    let url = format!("{}/device/register", cs_url);

    let resp = client
        .post(&url)
        .json(&DeviceRegisterRequest { hardware_fingerprint: fingerprint, machine_type })
        .send()
        .await
        .with_context(|| format!("Falha ao conectar ao CS em {}", url))?;

    let status = resp.status();
    if status.is_success() {
        resp.json::<DeviceRegisterResponse>().await.context("Falha ao decodificar resposta de device/register")
    } else {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("register_device HTTP {}: {}", status, body)
    }
}

/// POST /device/token — poll até aprovado. 429 (rate limit do próprio
/// servidor, ~3s entre polls) é tratado como "ainda pendente", não como erro
/// — quem decide o intervalo entre chamadas é o chamador (`run_device_flow`).
pub async fn poll_device_token(cs_url: &str, device_code: &str) -> Result<DeviceTokenResponse> {
    let client = build_client()?;
    let url = format!("{}/device/token", cs_url);

    let resp = client
        .post(&url)
        .json(&DeviceTokenRequest { device_code })
        .send()
        .await
        .with_context(|| format!("Falha ao conectar ao CS em {}", url))?;

    let status = resp.status();
    if status.as_u16() == 429 {
        return Ok(DeviceTokenResponse {
            status: "ok".to_string(),
            registration_status: "pending".to_string(),
            machine_code: None,
            token: None,
            rgs_url: None,
            rgs_port: None,
        });
    }
    if status.is_success() {
        resp.json::<DeviceTokenResponse>().await.context("Falha ao decodificar resposta de device/token")
    } else {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("poll_device_token HTTP {}: {}", status, body)
    }
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

#[cfg(test)]
mod device_flow_tests {
    use super::*;

    #[tokio::test]
    async fn register_device_parses_pending_response() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/device/register")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"ok","device_code":"abc123","user_code":"WXYZ-1234","registration_status":"pending","expires_in":900}"#)
            .create_async()
            .await;

        let resp = register_device(&server.url(), "fp-teste", "dual_screen").await.unwrap();
        assert_eq!(resp.device_code, "abc123");
        assert_eq!(resp.user_code, "WXYZ-1234");
    }

    #[tokio::test]
    async fn register_device_same_fingerprint_returns_existing_code() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/device/register")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"ok","device_code":"abc123","user_code":"WXYZ-1234","registration_status":"approved"}"#)
            .create_async()
            .await;

        let resp = register_device(&server.url(), "fp-ja-conhecido", "single_screen_vertical").await.unwrap();
        assert_eq!(resp.registration_status, "approved");
    }

    #[tokio::test]
    async fn poll_device_token_pending() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/device/token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"ok","registration_status":"pending"}"#)
            .create_async()
            .await;

        let resp = poll_device_token(&server.url(), "device-code-1").await.unwrap();
        assert_eq!(resp.registration_status, "pending");
        assert!(resp.token.is_none());
    }

    #[tokio::test]
    async fn poll_device_token_approved_has_token() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/device/token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"ok","registration_status":"approved","machine_code":"LAB-M-99","token":"TOK123","rgs_url":"http://rgs","rgs_port":43310}"#)
            .create_async()
            .await;

        let resp = poll_device_token(&server.url(), "device-code-1").await.unwrap();
        assert_eq!(resp.registration_status, "approved");
        assert_eq!(resp.machine_code.as_deref(), Some("LAB-M-99"));
        assert_eq!(resp.token.as_deref(), Some("TOK123"));
    }

    #[tokio::test]
    async fn poll_device_token_rate_limited_treated_as_pending() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/device/token")
            .with_status(429)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"fail","message":"polling too fast","retry_after":3}"#)
            .create_async()
            .await;

        // 429 não deve virar Err — o chamador (run_device_flow) trata como "ainda pendente"
        let resp = poll_device_token(&server.url(), "device-code-1").await.unwrap();
        assert_eq!(resp.registration_status, "pending");
    }

    #[tokio::test]
    async fn poll_device_token_not_found_is_error() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/device/token")
            .with_status(404)
            .with_body(r#"{"status":"fail","message":"device_code not found"}"#)
            .create_async()
            .await;

        let result = poll_device_token(&server.url(), "device-code-inexistente").await;
        assert!(result.is_err());
    }
}
