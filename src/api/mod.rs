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

#[derive(Debug, Clone, Deserialize)]
pub struct IslandInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoomInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocationInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct LookupCodeEnvelope {
    status: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    location: Option<LocationInfo>,
    #[serde(default)]
    room: Option<RoomInfo>,
    #[serde(default)]
    islands: Vec<IslandInfo>,
    #[serde(default = "default_machine_variant")]
    machine_variant: String,
}

fn default_machine_variant() -> String {
    "vlt".to_string()
}

/// Resultado de `GET /device/lookup_code` — a sala/local resolvidos a partir
/// da faixa numérica em que o código caiu, e as ilhas dessa sala pra TUI
/// oferecer como escolha (não existe dropdown de local/sala, só de ilha).
/// `machine_variant` (`vlt`/`street`) vem da sala — decide qual build o
/// launcher vai pedir ao game_registry_service mais tarde.
#[derive(Debug, Clone)]
pub struct RoomLookup {
    pub location: LocationInfo,
    pub room: RoomInfo,
    pub islands: Vec<IslandInfo>,
    pub machine_variant: String,
}

#[derive(Debug, Deserialize)]
pub struct DeviceActivateResponse {
    pub status: String,
    #[serde(default)]
    pub machine_code: Option<String>,
    #[serde(default)]
    pub display_label: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub rgs_url: Option<String>,
    #[serde(default)]
    pub rgs_port: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct LookupCodeRequest<'a> {
    machine_code: &'a str,
}

#[derive(Debug, Serialize)]
struct DeviceActivateRequest<'a> {
    machine_code: &'a str,
    hardware_fingerprint: &'a str,
    /// Autodetectado por `hardware::video::detect_machine_type()` — não é
    /// escolhido pelo operador (ver `hardware/video.rs`).
    machine_type: &'a str,
    id_island: &'a str,
    position: u32,
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

/// POST /device/lookup_code — resolve o código de 4 dígitos digitado pelo
/// operador pra sala/local (via faixa cadastrada na sala) + lista de ilhas
/// dessa sala. 404 = código fora de qualquer faixa cadastrada.
pub async fn lookup_code(cs_url: &str, machine_code: &str) -> Result<RoomLookup> {
    let client = build_client()?;
    let url = format!("{}/device/lookup_code", cs_url);

    let resp = client
        .post(&url)
        .json(&LookupCodeRequest { machine_code })
        .send()
        .await
        .with_context(|| format!("Falha ao conectar ao CS em {}", url))?;

    let status = resp.status();
    if status.is_success() {
        let envelope = resp
            .json::<LookupCodeEnvelope>()
            .await
            .context("Falha ao decodificar resposta de device/lookup_code")?;
        let location = envelope.location.context("Resposta de lookup_code sem location")?;
        let room = envelope.room.context("Resposta de lookup_code sem room")?;
        Ok(RoomLookup { location, room, islands: envelope.islands, machine_variant: envelope.machine_variant })
    } else {
        let body = resp
            .json::<LookupCodeEnvelope>()
            .await
            .ok()
            .and_then(|e| e.message)
            .unwrap_or_else(|| status.to_string());
        anyhow::bail!("lookup_code HTTP {}: {}", status, body)
    }
}

/// POST /device/activate — cria a máquina e ativa na hora, sem
/// backoffice/aprovação. Idempotente: reenviar o mesmo `machine_code` já
/// ativado (mesmo fingerprint) devolve o mesmo token em vez de duplicar.
pub async fn activate_device(
    cs_url: &str,
    machine_code: &str,
    fingerprint: &str,
    machine_type: &str,
    id_island: &str,
    position: u32,
) -> Result<DeviceActivateResponse> {
    let client = build_client()?;
    let url = format!("{}/device/activate", cs_url);

    let resp = client
        .post(&url)
        .json(&DeviceActivateRequest {
            machine_code,
            hardware_fingerprint: fingerprint,
            machine_type,
            id_island,
            position,
        })
        .send()
        .await
        .with_context(|| format!("Falha ao conectar ao CS em {}", url))?;

    let status = resp.status();
    if status.is_success() {
        resp.json::<DeviceActivateResponse>().await.context("Falha ao decodificar resposta de device/activate")
    } else {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("activate_device HTTP {}: {}", status, body)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameLatestResponse {
    pub game_type: String,
    #[allow(dead_code)]
    pub variant: String,
    pub version: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub download_url: String,
}

/// GET {registry}/game/latest?game_type=X&variant=Y — versão mais recente
/// publicada, com hash pra conferir depois do download.
pub async fn get_latest_build(registry_url: &str, game_type: &str, variant: &str) -> Result<GameLatestResponse> {
    let client = build_client()?;
    let url = format!("{}/game/latest", registry_url);

    let resp = client
        .get(&url)
        .query(&[("game_type", game_type), ("variant", variant)])
        .send()
        .await
        .with_context(|| format!("Falha ao conectar ao game_registry em {}", url))?;

    let status = resp.status();
    if status.is_success() {
        resp.json::<GameLatestResponse>().await.context("Falha ao decodificar resposta de game/latest")
    } else {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("get_latest_build HTTP {}: {}", status, body)
    }
}

/// Baixa o pacote inteiro pra memória (nunca disco) — builds ficam na casa
/// de centenas de MB, timeout maior que o client padrão de 30s.
/// Baixa em stream (nunca no disco, só acumula em memória) chamando
/// `on_progress(baixado_ate_agora, total_se_conhecido)` a cada chunk — quem
/// chama decide com que frequência desenha isso na tela.
pub async fn download_bytes_with_progress(
    url: &str,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>> {
    use futures_util::StreamExt;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .context("Falha ao criar cliente HTTP")?;

    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Falha ao baixar {}", url))?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("download HTTP {}", status);
    }

    let total = resp.content_length();
    let mut buf: Vec<u8> = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Falha ao ler chunk do download")?;
        buf.extend_from_slice(&chunk);
        on_progress(buf.len() as u64, total);
    }

    Ok(buf)
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
    async fn lookup_code_parses_room_and_islands() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/device/lookup_code")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"ok",
                     "location":{"id":"L1","name":"Stock"},
                     "room":{"id":"R1","name":"Sala 1"},
                     "islands":[{"id":"I1","name":"Ilha 1"},{"id":"I2","name":"Ilha 2"}]}"#,
            )
            .create_async()
            .await;

        let lookup = lookup_code(&server.url(), "8005").await.unwrap();
        assert_eq!(lookup.room.name, "Sala 1");
        assert_eq!(lookup.islands.len(), 2);
    }

    #[tokio::test]
    async fn lookup_code_out_of_range_is_error() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/device/lookup_code")
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"fail","message":"no room for this code"}"#)
            .create_async()
            .await;

        let result = lookup_code(&server.url(), "9999").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn activate_device_returns_token() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/device/activate")
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"status":"ok","machine_code":"8005","display_label":"STK-S1-I1-001",
                     "token":"TOK123","rgs_url":"http://rgs","rgs_port":43310}"#,
            )
            .create_async()
            .await;

        let resp = activate_device(&server.url(), "8005", "fp-teste", "dual_screen", "I1", 3)
            .await
            .unwrap();
        assert_eq!(resp.machine_code.as_deref(), Some("8005"));
        assert_eq!(resp.token.as_deref(), Some("TOK123"));
    }

    #[tokio::test]
    async fn activate_device_conflict_is_error() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/device/activate")
            .with_status(409)
            .with_body(r#"{"status":"fail","message":"code already used by another device"}"#)
            .create_async()
            .await;

        let result = activate_device(&server.url(), "8005", "fp-outra", "dual_screen", "I1", 1).await;
        assert!(result.is_err());
    }
}
