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

/// Comando que o backend manda de carona na resposta do heartbeat —
/// hoje só `update_launcher` existe. Backend decide quem recebe (1
/// máquina/ilha/sala/tudo, ver `AdminScheduleLauncherUpdate` no CS);
/// launcher só obedece o que chegar no seu próprio heartbeat.
/// `version`/`url`/`sha256` só vêm preenchidos pra `type: "update_launcher"`
/// — comandos avulsos (`reboot`/`restart_service`) mandam só `type`.
#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatCommand {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatResponse {
    #[serde(default)]
    pub command: Option<HeartbeatCommand>,
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
    ip_addresses: &'a str,
    // Componentes crus do fingerprint (ver `hardware::compute_fingerprint`) —
    // o hash combinado já vai em `hardware_fingerprint` acima; esses campos
    // são só pra auditoria (CS guarda de forma idempotente, não sobrescreve
    // se já tiver salvo — ver plano/backlog).
    bios_uuid: &'a str,
    baseboard_serial: &'a str,
    cpu_id: &'a str,
    disk_serials: &'a str,
    mac_address: &'a str,
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
    ip_addresses: &'a str,
    // Mesma ideia do device/activate — máquina já pareada antes dessa
    // feature existir nunca passa pelo device flow de novo, então manda
    // os componentes aqui também. CS grava só se ainda não tiver (COALESCE),
    // uma vez só, sem re-pareamento.
    bios_uuid: &'a str,
    baseboard_serial: &'a str,
    cpu_id: &'a str,
    disk_serials: &'a str,
    mac_address: &'a str,
}

#[derive(Debug, Serialize)]
struct GameEvent<'a> {
    code: &'a str,
    occurred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct HeartbeatRequest<'a> {
    machine_code: &'a str,
    ip_addresses: &'a str,
    events: Vec<GameEvent<'a>>,
}

fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Falha ao criar cliente HTTP")
}

/// IPs (v4) de todas as interfaces de rede, via `hostname -I` (já vem no
/// Debian) — inclui LAN, wifi, e o que mais tiver (tailscale etc), espaço
/// separado. Backend guarda como veio, sem parsear.
/// ponytail: shell out em vez de lib de rede, uma linha resolve.
pub fn local_ips() -> String {
    std::process::Command::new("hostname")
        .arg("-I")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
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

pub async fn get_token(cs_url: &str, machine_code: &str, fingerprint: &str, hw: &crate::hardware::HardwareInfo) -> Result<TokenResponse> {
    let client = build_client()?;
    let url = format!("{}/launcher", cs_url);
    let disk_serials = hw.disk_serials.join(",");

    let resp = client
        .post(&url)
        .json(&LauncherRequest {
            machine_code,
            hardware_fingerprint: fingerprint,
            ip_addresses: &local_ips(),
            bios_uuid: &hw.bios_uuid,
            baseboard_serial: &hw.baseboard_serial,
            cpu_id: &hw.cpu_id,
            disk_serials: &disk_serials,
            mac_address: &hw.mac_address,
        })
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
    hw: &crate::hardware::HardwareInfo,
) -> Result<DeviceActivateResponse> {
    let client = build_client()?;
    let url = format!("{}/device/activate", cs_url);
    let disk_serials = hw.disk_serials.join(",");

    let resp = client
        .post(&url)
        .json(&DeviceActivateRequest {
            machine_code,
            hardware_fingerprint: fingerprint,
            machine_type,
            id_island,
            position,
            ip_addresses: &local_ips(),
            bios_uuid: &hw.bios_uuid,
            baseboard_serial: &hw.baseboard_serial,
            cpu_id: &hw.cpu_id,
            disk_serials: &disk_serials,
            mac_address: &hw.mac_address,
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
    #[allow(dead_code)]
    pub layout: String,
    pub version: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub download_url: String,
}

/// GET {registry}/game/latest?game_type=X&variant=Y&layout=Z — versão mais
/// recente publicada, com hash pra conferir depois do download. `layout`
/// vem de `hardware::video::detect_machine_type()` (`dual_screen` /
/// `single_screen_vertical`) — cada quantidade de tela tem build própria.
pub async fn get_latest_build(registry_url: &str, game_type: &str, variant: &str, layout: &str) -> Result<GameLatestResponse> {
    let client = build_client()?;
    let url = format!("{}/game/latest", registry_url);

    let resp = client
        .get(&url)
        .query(&[("game_type", game_type), ("variant", variant), ("layout", layout)])
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

/// POST {cs_url}/machine/game_version_report — reporta a versão do jogo
/// que rodou. Dois casos: (1) 1º boot pós-pareamento sem pin ainda
/// (`status="success"`, vira o pin baseline no CS); (2) depois de aplicar
/// update sob demanda (comando `update_game` do heartbeat). Sem HMAC, mesmo
/// modelo de confiança do heartbeat/launcher_update_report — best-effort,
/// não trava o boot se o CS estiver fora do ar.
pub async fn report_game_version(cs_url: &str, machine_code: &str, version: &str, status: &str) {
    let client = match build_client() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Falha ao criar client HTTP pra reportar versão do jogo: {}", e);
            return;
        }
    };
    let url = format!("{}/machine/game_version_report", cs_url);
    let body = serde_json::json!({
        "machine_code": machine_code,
        "version": version,
        "status": status,
    });
    if let Err(e) = client.post(&url).json(&body).send().await {
        log::warn!("Falha ao reportar versão do jogo pro backend: {}", e);
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LauncherLatestResponse {
    pub version: String,
    pub sha256: String,
    #[allow(dead_code)]
    pub size_bytes: u64,
    pub download_url: String,
}

/// GET {registry}/launcher/latest — versão mais recente do próprio
/// launcher publicada. Usado pelo botão "Atualizar Launcher" do menu
/// (máquina ainda sem pareamento, sem heartbeat pra receber update via
/// backend — checagem sob demanda, só quando o operador clica).
pub async fn get_latest_launcher_build(registry_url: &str) -> Result<LauncherLatestResponse> {
    let client = build_client()?;
    let url = format!("{}/launcher/latest", registry_url);

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Falha ao conectar ao game_registry em {}", url))?;

    let status = resp.status();
    if status.is_success() {
        resp.json::<LauncherLatestResponse>().await.context("Falha ao decodificar resposta de launcher/latest")
    } else {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("get_latest_launcher_build HTTP {}: {}", status, body)
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

/// Devolve o comando pendente (se tiver algum) na resposta do heartbeat —
/// ver `HeartbeatCommand`. Aplicar o comando é responsabilidade de quem
/// chama, não desse módulo de API.
pub async fn send_heartbeat(
    cs_url: &str,
    machine_code: &str,
    events: &[(String, chrono::DateTime<chrono::Utc>)],
) -> Result<Option<HeartbeatCommand>> {
    let client = build_client()?;
    let url = format!("{}/machine/heartbeat", cs_url);

    let events = events
        .iter()
        .map(|(code, occurred_at)| GameEvent { code, occurred_at: *occurred_at })
        .collect();

    let resp = client
        .post(&url)
        .json(&HeartbeatRequest { machine_code, ip_addresses: &local_ips(), events })
        .send()
        .await
        .with_context(|| format!("Falha ao enviar heartbeat para {}", url))?;

    if resp.status().is_success() {
        let body = resp.json::<HeartbeatResponse>().await.context("Falha ao decodificar resposta de heartbeat")?;
        Ok(body.command)
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

        let hw = crate::hardware::HardwareInfo {
            mac_address: "aa:bb".to_string(),
            bios_uuid: "uuid-1".to_string(),
            baseboard_serial: "board-1".to_string(),
            cpu_id: "cpu-1".to_string(),
            disk_serials: vec!["disk-1".to_string()],
            processor: "test-cpu".to_string(),
            hostname: "test-host".to_string(),
            serial_number: None,
        };
        let resp = activate_device(&server.url(), "8005", "fp-teste", "dual_screen", "I1", 3, &hw)
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

        let hw = crate::hardware::HardwareInfo {
            mac_address: "aa:bb".to_string(),
            bios_uuid: "uuid-1".to_string(),
            baseboard_serial: "board-1".to_string(),
            cpu_id: "cpu-1".to_string(),
            disk_serials: vec!["disk-1".to_string()],
            processor: "test-cpu".to_string(),
            hostname: "test-host".to_string(),
            serial_number: None,
        };
        let result = activate_device(&server.url(), "8005", "fp-outra", "dual_screen", "I1", 1, &hw).await;
        assert!(result.is_err());
    }
}
