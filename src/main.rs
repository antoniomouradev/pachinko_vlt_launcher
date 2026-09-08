use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::event::KeyCode;
use log::{info, warn, error};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

/// PID do processo do jogo em execução agora, se houver — permite ao
/// `heartbeat_loop` (rodando em task separada) matar o jogo sob comando
/// `update_game` sem precisar guardar o `Child` inteiro (que ficaria preso
/// no `.wait()` bloqueante de `spawn_game_and_wait`). `kill <pid>` externo
/// via shell em vez de `Child::kill()` — evita disputa de lock em torno do
/// `.wait()`.
type SharedGamePid = Arc<Mutex<Option<u32>>>;

/// Handle da task de `heartbeat_loop` em execução — achado real 28/07: toda
/// vez que o jogo reinicia (crash-loop), `try_get_token_and_run` subia um
/// `heartbeat_loop` **novo** sem nunca cancelar o anterior. Numa máquina com
/// o jogo falhando repetido, isso empilhava dezenas de heartbeats
/// concorrentes — resultado visto ao vivo: comando batendo a cada poucos
/// segundos em vez de a cada 30s, disputando rede entre si. Abortar o
/// anterior antes de subir um novo garante só 1 vivo por vez.
type SharedHeartbeatHandle = Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>;

fn respawn_heartbeat_loop(
    handle_slot: &SharedHeartbeatHandle,
    cs_url: String,
    machine_code: String,
    config_path: PathBuf,
    game_pid: SharedGamePid,
    game_status: status_listener::GameStatusChannels,
) {
    let mut slot = handle_slot.lock().unwrap();
    if let Some(old) = slot.take() {
        old.abort();
    }
    *slot = Some(tokio::spawn(async move {
        heartbeat_loop(&cs_url, &machine_code, &config_path, game_pid, game_status).await;
    }));
}

mod hardware;
mod api;
mod config;
mod game_runtime;
mod service;
mod env_config;
mod error;
mod network_selfheal;
mod setup;
mod status_listener;
mod tui;
mod update;

use config::{LauncherConfig, LauncherSettings, load_config, save_config, delete_config, get_config_path, get_pairing_code_path, get_settings_path, load_settings};

const HEARTBEAT_INTERVAL_SECS: u64 = 30;
/// Canal separado do heartbeat, de propósito: heartbeat é liveness + código
/// mais recente (leve, frequente); esse aqui manda o lote completo de
/// transições do jogo pro histórico (`machine_event`), sem pressa.
const GAME_EVENTS_INTERVAL_SECS: u64 = 120;
#[allow(dead_code)] // usado só no fluxo antigo (wait_for_pairing), ver comentário lá
const PAIRING_POLL_INTERVAL_SECS: u64 = 10;
const DEVICE_FLOW_RETRY_SECS: u64 = 10;
const GAME_RESTART_DELAY_SECS: u64 = 3;
/// VT (console de texto) onde o launcher roda — `TTYPath` da unit systemd.
const LAUNCHER_VT: &str = "1";
/// VT onde o X/jogo roda — ver `.xinitrc`/autologin do usuário `game`.
const GAME_VT: &str = "2";
/// X e jogo rodam como root (ver `ensure_x_running`) — display/cookie fixos.
const X_DISPLAY: &str = ":0";
const X_AUTHORITY: &str = "/root/.Xauthority";
/// Resolução por tela do jogo hoje (fixo — quando existir variante 1 tela,
/// isso deixa de ser constante única, ver comentário em `configure_display_layout`).
const GAME_SCREEN_MODE: &str = "800x600";
const SINGLE_SCREEN_MODE: &str = "1920x1080";

/// Único game_type suportado por enquanto (decisão do usuário) — quando
/// entrar outro jogo, isso vira campo vindo da ilha em vez de constante.
const GAME_TYPE: &str = "pachinko3";

#[derive(Parser)]
#[command(name = "pachinko_vlt_launcher")]
#[command(about = "Launcher de máquinas Pachinko")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Configura o launcher pela primeira vez
    Setup {
        /// URL do servidor CS (ex: http://192.168.1.10:8888)
        #[arg(long)]
        cs_url: Option<String>,
        /// Caminho do jogo ou lobby
        #[arg(long)]
        game: Option<String>,
        /// Argumentos extras para o jogo (entre aspas: "--env=local --channel web")
        #[arg(long)]
        game_args: Option<String>,
        /// Código de pareamento de 8 caracteres
        #[arg(long)]
        pairing_code: Option<String>,
    },
    /// Instala launcher como serviço do sistema
    InstallService,
    /// Remove launcher do sistema
    UninstallService,
}

/// Onde os logs do próprio launcher (info!/warn!/error!) ficam gravados —
/// terminal sozinho não serve porque a TUI (raw mode/alt screen) sobrescreve
/// tudo, some assim que a tela redesenha.
const LAUNCHER_LOG_PATH: &str = "/var/log/pachinko-launcher.log";
/// stdout+stderr do processo do jogo em si (SDL/ALSA/crash) — separado do
/// log do launcher pra não misturar as duas coisas.
const GAME_LOG_PATH: &str = "/var/log/pachinko-launcher-game.log";

/// Loga em arquivo além do terminal (quando dá pra abrir o arquivo — se não
/// der, cai só no terminal mesmo, não trava o launcher por causa de log).
fn init_logging() {
    use std::io::Write;

    let file = std::fs::OpenOptions::new().create(true).append(true).open(LAUNCHER_LOG_PATH).ok();

    let mut builder = env_logger::Builder::from_default_env();
    builder.filter_level(log::LevelFilter::Info);
    if let Some(file) = file {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
        builder.format(|buf, record| {
            writeln!(buf, "[{}] {} - {}", chrono::Utc::now().to_rfc3339(), record.level(), record.args())
        });
    }
    builder.init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Setup { cs_url, game, game_args, pairing_code }) => {
            setup::run_setup(setup::SetupArgs {
                cs_url,
                game_path: game,
                game_args,
                pairing_code,
            })
        }
        Some(Commands::InstallService) => {
            service::install_service().context("Falha ao instalar serviço")?;
            info!("Serviço instalado com sucesso");
            Ok(())
        }
        Some(Commands::UninstallService) => {
            service::uninstall_service().context("Falha ao remover serviço")?;
            info!("Serviço removido com sucesso");
            Ok(())
        }
        None => run().await,
    }
}

fn load_runtime_settings() -> LauncherSettings {
    if let Ok(path) = get_settings_path() {
        if let Ok(s) = load_settings(&path) {
            return s;
        }
    }
    LauncherSettings {
        cs_url: std::env::var("CS_URL").unwrap_or_else(|_| env_config::default_cs_url().to_string()),
        game_path: std::env::var("VLT_GAME_PATH").unwrap_or_else(|_| "./pachinko_game".to_string()),
        // Mesmo comando já usado em produção noutra máquina (ver .xinitrc de
        // referência) — só o `--token` virou `GAME_TOKEN` via env.
        game_args: std::env::var("VLT_GAME_ARGS")
            .unwrap_or_else(|_| {
                "-release -- --env=prod --channel web --layout=stacked_dual --button-hub --top-header-font".to_string()
            })
            .split_whitespace()
            .map(String::from)
            .collect(),
        game_registry_url: std::env::var("GAME_REGISTRY_URL")
            .unwrap_or_else(|_| env_config::default_game_registry_url().to_string()),
    }
}

async fn run() -> Result<()> {
    let settings = load_runtime_settings();
    let cs_url = settings.cs_url.clone();

    hardware::audio::set_volume(env_config::default_volume_percent());
    hardware::touch::ensure_touch_calibration();

    let hw_info = hardware::collect()
        .context("Falha ao coletar informações de hardware")?;
    let fingerprint = hardware::compute_fingerprint(&hw_info);
    info!("Hardware fingerprint: {}", &fingerprint[..20]);

    let config_path = get_config_path()
        .context("Não foi possível determinar caminho de configuração")?;

    // Sem marcador de update pendente, no-op — só age de verdade logo depois
    // de um `apply_update` ter trocado o symlink (self-check confirma ou
    // reverte sozinho antes de seguir pro menu/jogo).
    let machine_code_for_check = load_config(&config_path).ok().map(|c| c.machine_code);
    update::check_pending_update_or_rollback(&cs_url, machine_code_for_check.as_deref()).await;

    let game_pid: SharedGamePid = Arc::new(Mutex::new(None));
    let heartbeat_handle: SharedHeartbeatHandle = Arc::new(Mutex::new(None));

    let game_status = status_listener::GameStatusChannels::new();
    tokio::spawn(status_listener::run(game_status.clone(), status_listener::DEFAULT_PORT));

    loop {
        let config = load_config(&config_path).ok();

        match config {
            Some(cfg) => {
                match try_get_token_and_run(&cs_url, &cfg, &fingerprint, &hw_info, &config_path, &settings, game_pid.clone(), heartbeat_handle.clone(), game_status.clone()).await {
                    Ok(()) => {
                        // Sem isso, jogo que crasha na hora (SDL/lib faltando/etc)
                        // vira loop bem apertado: reinicia sem pausa nenhuma, cada
                        // ciclo só bate `/launcher` + `/game/latest` de novo — no
                        // registry parece um retry de rede martelando, mas é o
                        // jogo caindo repetido (ver `game.log`/stderr do processo
                        // pra causa raiz de verdade).
                        warn!("Jogo encerrado. Reiniciando em {}s...", GAME_RESTART_DELAY_SECS);
                        sleep(Duration::from_secs(GAME_RESTART_DELAY_SECS)).await;
                    }
                    Err(e) if e.to_string().contains("401") || e.to_string().contains("403") => {
                        warn!("Credenciais inválidas ({}). Limpando config e reiniciando device flow.", e);
                        delete_config(&config_path);
                        run_device_flow(&cs_url, &fingerprint, &hw_info, &config_path, &settings, game_pid.clone(), heartbeat_handle.clone(), game_status.clone()).await?;
                    }
                    Err(e) => {
                        // `{:?}` (não `{}`) — no anyhow::Error o `{}` só mostra o
                        // `.context()` de topo, escondendo a causa real (hash
                        // mismatch, disco cheio etc.) na chain de baixo (achado
                        // real 19/08: log só dizia "Falha ao preparar build do
                        // jogo", sem detalhe nenhum, até investigar na unha).
                        error!("Erro ao obter token: {:?}. Tentando novamente em {}s...", e, HEARTBEAT_INTERVAL_SECS);
                        // Erro de conexão (não 401/403, já tratado acima) —
                        // tenta consertar rede sozinho antes do próximo retry
                        // (ver network_selfheal.rs pro achado real).
                        if e.to_string().contains("Falha ao conectar") {
                            network_selfheal::try_self_heal();
                        }
                        sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
                    }
                }
            }
            None => {
                // Máquina crua (sem config ainda) — mostra o menu inicial em vez de
                // ir direto pro registro. "Registrar Máquina" usa o device flow
                // (código de 4 dígitos + faixa da sala), não o pareamento OTP antigo.
                match tui::run_menu()? {
                    Some(tui::MenuChoice::ConfigureMachine) => {
                        run_device_flow(&cs_url, &fingerprint, &hw_info, &config_path, &settings, game_pid.clone(), heartbeat_handle.clone(), game_status.clone()).await?;
                    }
                    Some(tui::MenuChoice::TestMachine) => {
                        run_test_menu_loop().await?;
                    }
                    Some(tui::MenuChoice::UpdateLauncher) => {
                        run_update_launcher_from_menu(&settings).await?;
                    }
                    Some(tui::MenuChoice::Restart) => {
                        info!("Reiniciado pelo menu — chamando systemctl reboot.");
                        // .status() (espera terminar), não .spawn() — achado
                        // real: com spawn() o launcher retornava Ok(()) e
                        // morria (exit 0, `Restart=on-failure` não reinicia
                        // em saída limpa) quase junto, e o systemd
                        // (KillMode=control-group, padrão) matava o
                        // `systemctl reboot` no meio do caminho antes dele
                        // completar — máquina nunca reiniciava de verdade.
                        // `systemctl reboot` só manda o pedido pro PID1 e
                        // retorna rápido, não espera o reboot física
                        // acontecer — `.status()` não trava.
                        let _ = Command::new("systemctl").arg("reboot").status();
                        return Ok(());
                    }
                    Some(tui::MenuChoice::Shutdown) => {
                        info!("Desligado pelo menu — chamando systemctl poweroff.");
                        // Mesmo achado do Restart acima — .status() em vez
                        // de .spawn(), garante que o pedido chegou no PID1
                        // antes do processo morrer.
                        let _ = Command::new("systemctl").arg("poweroff").status();
                        return Ok(());
                    }
                    None => {}
                }
            }
        }
    }
}

async fn try_get_token_and_run(
    cs_url: &str,
    config: &LauncherConfig,
    fingerprint: &str,
    hw_info: &hardware::HardwareInfo,
    config_path: &PathBuf,
    settings: &LauncherSettings,
    game_pid: SharedGamePid,
    heartbeat_handle: SharedHeartbeatHandle,
    game_status: status_listener::GameStatusChannels,
) -> Result<()> {
    info!("Obtendo token para máquina {}...", config.machine_code);
    let token_resp = api::get_token(cs_url, &config.machine_code, fingerprint, hw_info).await?;

    info!("Token obtido. Iniciando jogo...");
    respawn_heartbeat_loop(
        &heartbeat_handle,
        cs_url.to_string(),
        config.machine_code.clone(),
        config_path.clone(),
        game_pid.clone(),
        game_status,
    );

    let rgs_url = build_rgs_url(&token_resp.rgs_url, &token_resp.rgs_port);
    let exit_status =
        spawn_game_and_wait(&token_resp.token, rgs_url.as_deref(), config, config_path, settings, game_pid).await?;
    info!("Jogo encerrado (status: {}). Reiniciando...", exit_status);

    Ok(())
}

/// Botão "Atualizar Launcher" do menu — sob demanda, só quando o operador
/// clica (máquina ainda sem pareamento não tem heartbeat, não recebe
/// update via backend). Consulta a versão mais recente publicada direto
/// no registry, reusa a mesma lógica de baixar/conferir/trocar/reiniciar
/// do update via heartbeat (`update::apply_update`).
async fn run_update_launcher_from_menu(settings: &LauncherSettings) -> Result<()> {
    let mut screen = tui::enter_screen()?;
    tui::draw_lines(&mut screen, "Atualizar Launcher", &["Consultando versão mais recente...".to_string()])?;

    let latest = match api::get_latest_launcher_build(&settings.game_registry_url).await {
        Ok(latest) => latest,
        Err(e) => {
            tui::leave_screen(screen)?;
            tui::show_placeholder("Atualizar Launcher", &format!("Falha ao consultar registry: {}", e))?;
            return Ok(());
        }
    };

    if latest.version == env!("CARGO_PKG_VERSION") {
        tui::leave_screen(screen)?;
        tui::show_placeholder("Atualizar Launcher", &format!("Já está na última versão ({}).", latest.version))?;
        return Ok(());
    }

    tui::draw_lines(
        &mut screen,
        "Atualizar Launcher",
        &[format!("Baixando e aplicando versão {}...", latest.version)],
    )?;
    if let Err(e) = update::apply_update(&latest.version, &latest.download_url, &latest.sha256).await {
        tui::leave_screen(screen)?;
        tui::show_placeholder("Atualizar Launcher", &format!("Falha ao atualizar: {}", e))?;
        return Ok(());
    }

    // Deu certo — `apply_update` já disparou `systemctl restart`, que mata
    // este processo em instantes. Sem mais nada a fazer aqui.
    Ok(())
}

/// Fica no submenu "Testar Máquina" até o usuário apertar Esc/q.
async fn run_test_menu_loop() -> Result<()> {
    loop {
        match tui::run_test_menu()? {
            Some(tui::TestChoice::Connection) => {
                run_connection_test_loop().await?;
            }
            Some(tui::TestChoice::Audio) => {
                run_audio_test()?;
            }
            Some(tui::TestChoice::Video) => {
                let displays = hardware::video::get_displays();
                tui::run_video_test(&displays, Duration::from_millis(900))?;
            }
            Some(tui::TestChoice::Inputs) => run_buttonhub_test("Testar Inputs")?,
            None => return Ok(()),
        }
    }
}

/// Fica pingando (checagem de internet, não é ICMP) em loop até Esc/q.
/// `check_connection` já tem timeout de 5s — enquanto uma tentativa está
/// pendurada (rede fora do ar), a tecla de saída só é lida depois que essa
/// tentativa retorna, ou seja, sair pode demorar até ~5s nesse cenário.
async fn run_connection_test_loop() -> Result<()> {
    let mut screen = tui::enter_screen()?;
    let mut history: Vec<String> = Vec::new();
    let ips = {
        let s = api::local_ips();
        if s.is_empty() { "nenhum".to_string() } else { s }
    };

    let result: Result<()> = loop {
        let line = match api::check_connection().await {
            Ok(elapsed) => format!(
                "[{}] OK — {}ms",
                chrono::Local::now().format("%H:%M:%S"),
                elapsed.as_millis()
            ),
            Err(e) => format!("[{}] FALHOU — {}", chrono::Local::now().format("%H:%M:%S"), e),
        };
        history.push(line);
        if history.len() > 20 {
            history.remove(0);
        }

        let mut lines = vec![
            "Ping contínuo — Esc para sair".to_string(),
            format!("IP(s): {}", ips),
            String::new(),
        ];
        lines.extend(history.iter().cloned());
        if let Err(e) = tui::draw_lines(&mut screen, "Conexão", &lines) {
            break Err(e);
        }

        match tui::poll_key(Duration::from_secs(1)) {
            Ok(Some(KeyCode::Esc)) => break Ok(()),
            Ok(_) => {}
            Err(e) => break Err(e),
        }
    };

    tui::leave_screen(screen)?;
    result
}

/// Conecta no `buttonhub` (teclas/chaves/noteiro, tudo pelo mesmo TCP — ver
/// interfaces/INTERFACE.md) e mostra cru cada evento que chegar.
fn run_buttonhub_test(title: &str) -> Result<()> {
    match hardware::buttonhub::connect(hardware::buttonhub::DEFAULT_PORT) {
        Ok(conn) => {
            let result = tui::run_event_stream(title, &conn.events);
            conn.close();
            result
        }
        Err(e) => tui::show_placeholder(
            title,
            &format!(
                "Falha ao conectar no buttonhub (porta {}): {}",
                hardware::buttonhub::DEFAULT_PORT,
                e
            ),
        ),
    }
}

/// Lista as saídas de áudio, deixa escolher uma e toca um tom de 440Hz por
/// 2s nela — pra confirmar que o som sai por aquela saída física.
fn run_audio_test() -> Result<()> {
    let devices = match hardware::audio::list_output_devices() {
        Ok(d) if !d.is_empty() => d,
        Ok(_) => {
            tui::show_placeholder("Som", "Nenhuma saída de áudio encontrada")?;
            return Ok(());
        }
        Err(e) => {
            tui::show_placeholder("Som", &format!("Erro ao listar saídas: {}", e))?;
            return Ok(());
        }
    };

    let items: Vec<&str> = devices.iter().map(String::as_str).collect();
    if let Some(idx) = tui::select("Som — escolha a saída", &items)? {
        let device_name = &devices[idx];
        let message = match hardware::audio::play_test_tone(device_name, Duration::from_secs(2)) {
            Ok(()) => format!("Tom de 440Hz tocado em \"{}\". Ouviu?", device_name),
            Err(e) => format!("Falha ao tocar tom em \"{}\": {}", device_name, e),
        };
        tui::show_placeholder("Som", &message)?;
    }

    Ok(())
}

/// Fluxo antigo (pareamento OTP manual via arquivo `pairing_code`) — não é
/// mais chamado por nada (`run_device_flow` substituiu nas duas chamadas que
/// existiam). Mantido sem apagar até a Etapa 6 (deprecar de vez, junto com o
/// lado servidor em `/machine/pair`) — servidor ainda aceita esse fluxo.
#[allow(dead_code)]
async fn wait_for_pairing(cs_url: &str, fingerprint: &str, config_path: &PathBuf, settings: &LauncherSettings) -> Result<()> {
    let pairing_file = get_pairing_code_path()
        .context("Não foi possível determinar caminho do pairing_code")?;

    loop {
        let pairing_code = std::env::var("PAIRING_CODE")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::fs::read_to_string(&pairing_file)
                    .ok()
                    .map(|s| s.trim().to_uppercase())
                    .filter(|s| !s.is_empty())
            });

        match pairing_code {
            Some(code) => {
                info!("Código de pareamento encontrado. Pareando máquina...");
                match api::pair_machine(cs_url, &code, fingerprint).await {
                    Ok(resp) => {
                        let cfg = LauncherConfig {
                            machine_code: resp.machine_code.clone(),
                            hardware_fingerprint: fingerprint.to_string(),
                            cs_url: cs_url.to_string(),
                            paired_at: chrono::Utc::now().to_rfc3339(),
                            // Fluxo antigo (OTP) sempre foi só pra VLT — nunca teve
                            // conceito de sala/variante, "vlt" é o único valor que
                            // fazia sentido aqui de qualquer forma.
                            machine_variant: "vlt".to_string(),
                            pinned_game_version: None,
                            pinned_game_sha256: None,
                            pinned_game_layout: None,
                            awaiting_layout_confirmation: false,
                        };
                        save_config(config_path, &cfg)
                            .context("Falha ao salvar configuração após pareamento")?;
                        let _ = std::fs::remove_file(&pairing_file);
                        let _ = std::env::remove_var("PAIRING_CODE");
                        info!("Máquina pareada: {}", resp.machine_code);

                        let game_pid: SharedGamePid = Arc::new(Mutex::new(None));
                        let heartbeat_handle: SharedHeartbeatHandle = Arc::new(Mutex::new(None));
                        let game_status = status_listener::GameStatusChannels::new();
                        respawn_heartbeat_loop(&heartbeat_handle, cs_url.to_string(), resp.machine_code.clone(), config_path.clone(), game_pid.clone(), game_status);

                        let rgs_url = build_rgs_url(&resp.rgs_url, &resp.rgs_port);
                        let exit_status =
                            spawn_game_and_wait(&resp.token, rgs_url.as_deref(), &cfg, config_path, settings, game_pid).await?;
                        info!("Jogo encerrado (status: {}). Reiniciando...", exit_status);
                        return Ok(());
                    }
                    Err(e) => {
                        error!("Falha no pareamento: {}. Tentando novamente em {}s...", e, PAIRING_POLL_INTERVAL_SECS);
                        sleep(Duration::from_secs(PAIRING_POLL_INTERVAL_SECS)).await;
                    }
                }
            }
            None => {
                info!("Aguardando código de pareamento em {} ou env PAIRING_CODE...", pairing_file.display());
                sleep(Duration::from_secs(PAIRING_POLL_INTERVAL_SECS)).await;
            }
        }
    }
}

/// Espera até `duration`, checando Esc a cada 200ms (mesmo padrão de
/// `run_video_test`). Retorna `true` se o operador cancelou.
fn wait_or_cancel(duration: Duration) -> Result<bool> {
    let deadline = std::time::Instant::now() + duration;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        if let Some(KeyCode::Esc) = tui::poll_key(remaining.min(Duration::from_millis(200)))? {
            return Ok(true);
        }
    }
}

/// Sinaliza a thread de `wait_for_escape` pra parar quando o `select!` que a
/// chama escolhe o outro branch — sem isso, a `spawn_blocking` continuaria
/// pollando tecla pra sempre numa thread solta (nunca recebe aviso de
/// cancelamento sozinha).
struct EscapeWatcherGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for EscapeWatcherGuard {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Fica escutando Esc indefinidamente, sem prazo — usado em telas de
/// preparação (download do jogo) onde não dá pra saber quanto tempo vai
/// levar. `poll_key` é síncrono/bloqueante, roda inteiro numa única
/// `spawn_blocking` (mesma thread do início ao fim — thread_local do
/// buttonhub não fica reconectando a cada 200ms).
async fn wait_for_escape() {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _guard = EscapeWatcherGuard(stop.clone());

    let handle = tokio::task::spawn_blocking(move || {
        loop {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            if let Ok(Some(KeyCode::Esc)) = tui::poll_key(Duration::from_millis(200)) {
                return;
            }
        }
    });

    let _ = handle.await;
}

/// Device flow (Etapa 5, revisado 2026-07-21): sem backoffice, sem
/// aprovação de admin. Quem instala digita só o código de 4 dígitos da
/// máquina (faixa numérica pré-atribuída à sala resolve local/sala do lado
/// do servidor, ver `device_lookup.py`), escolhe a ilha dentre as que já
/// existem na sala resolvida e digita a posição livre. Ativa na hora.
/// Rastreio de quem/quando instalou fica pra uma segunda camada de
/// autenticação futura (decisão do usuário, registrada em
/// `ROADMAP_LAUNCHER_DEVICE_FLOW.md`).
async fn run_device_flow(
    cs_url: &str,
    fingerprint: &str,
    hw_info: &hardware::HardwareInfo,
    config_path: &PathBuf,
    settings: &LauncherSettings,
    game_pid: SharedGamePid,
    heartbeat_handle: SharedHeartbeatHandle,
    game_status: status_listener::GameStatusChannels,
) -> Result<()> {
    let machine_type = hardware::video::detect_machine_type();

    let Some(machine_code) = tui::enter_digits("Código da máquina", 4)? else {
        return Ok(());
    };

    let mut screen = tui::enter_screen()?;
    let lookup = loop {
        tui::draw_lines(&mut screen, "Registrar Máquina", &["Consultando código...".to_string()])?;
        match api::lookup_code(cs_url, &machine_code).await {
            Ok(l) if l.islands.is_empty() => {
                tui::draw_lines(
                    &mut screen,
                    "Registrar Máquina",
                    &[
                        format!("{} / {} não tem ilha cadastrada ainda.", l.location.name, l.room.name),
                        format!("Tentando de novo em {}s... (Esc cancela)", DEVICE_FLOW_RETRY_SECS),
                    ],
                )?;
                if wait_or_cancel(Duration::from_secs(DEVICE_FLOW_RETRY_SECS))? {
                    tui::leave_screen(screen)?;
                    return Ok(());
                }
            }
            Ok(l) => break l,
            Err(e) => {
                tui::draw_lines(
                    &mut screen,
                    "Registrar Máquina",
                    &[
                        "Código inválido ou falha ao consultar:".to_string(),
                        e.to_string(),
                        String::new(),
                        "Esc volta e deixa digitar de novo.".to_string(),
                    ],
                )?;
                if wait_or_cancel(Duration::from_secs(DEVICE_FLOW_RETRY_SECS))? {
                    tui::leave_screen(screen)?;
                    return Ok(());
                }
            }
        }
    };
    tui::leave_screen(screen)?;

    let island_names: Vec<&str> = lookup.islands.iter().map(|i| i.name.as_str()).collect();
    let Some(island_idx) = tui::select(
        &format!("{} / {} — Escolha a ilha", lookup.location.name, lookup.room.name),
        &island_names,
    )? else {
        return Ok(());
    };
    let id_island = lookup.islands[island_idx].id.clone();
    let machine_variant = lookup.machine_variant.clone();

    let Some(position_str) = tui::enter_digits("Posição da máquina na ilha", 2)? else {
        return Ok(());
    };
    let position: u32 = position_str.parse().unwrap_or(0);

    let mut screen = tui::enter_screen()?;
    let approved = loop {
        tui::draw_lines(&mut screen, "Registrar Máquina", &["Ativando máquina...".to_string()])?;
        match api::activate_device(cs_url, &machine_code, fingerprint, machine_type, &id_island, position, hw_info).await {
            Ok(resp) => break resp,
            Err(e) => {
                tui::draw_lines(
                    &mut screen,
                    "Registrar Máquina",
                    &[
                        "Falha ao ativar:".to_string(),
                        e.to_string(),
                        format!("Tentando de novo em {}s... (Esc cancela)", DEVICE_FLOW_RETRY_SECS),
                    ],
                )?;
                if wait_or_cancel(Duration::from_secs(DEVICE_FLOW_RETRY_SECS))? {
                    tui::leave_screen(screen)?;
                    return Ok(());
                }
            }
        }
    };
    tui::leave_screen(screen)?;

    finish_device_flow(cs_url, fingerprint, config_path, settings, approved, machine_variant, game_pid, heartbeat_handle, game_status).await
}

async fn finish_device_flow(
    cs_url: &str,
    fingerprint: &str,
    config_path: &PathBuf,
    settings: &LauncherSettings,
    approved: api::DeviceActivateResponse,
    machine_variant: String,
    game_pid: SharedGamePid,
    heartbeat_handle: SharedHeartbeatHandle,
    game_status: status_listener::GameStatusChannels,
) -> Result<()> {
    let machine_code = approved
        .machine_code
        .context("Resposta de ativação sem machine_code")?;
    let token = approved.token.context("Resposta de ativação sem token")?;

    let cfg = LauncherConfig {
        machine_code: machine_code.clone(),
        hardware_fingerprint: fingerprint.to_string(),
        cs_url: cs_url.to_string(),
        paired_at: chrono::Utc::now().to_rfc3339(),
        machine_variant,
        // Sem pin ainda — 1º boot pós-pareamento busca `/game/latest` como
        // hoje e reporta a versão resolvida pro CS, que vira o pin baseline
        // (ver `spawn_game_and_wait`).
        pinned_game_version: None,
        pinned_game_sha256: None,
        pinned_game_layout: None,
        awaiting_layout_confirmation: false,
    };
    save_config(config_path, &cfg).context("Falha ao salvar configuração após aprovação")?;
    info!("Máquina ativada via device flow: {}", machine_code);

    respawn_heartbeat_loop(&heartbeat_handle, cs_url.to_string(), machine_code.clone(), config_path.clone(), game_pid.clone(), game_status);

    let rgs_url = approved
        .rgs_url
        .as_deref()
        .and_then(|url| build_rgs_url(url, approved.rgs_port.as_ref().unwrap_or(&serde_json::Value::Null)));
    let exit_status = spawn_game_and_wait(&token, rgs_url.as_deref(), &cfg, config_path, settings, game_pid).await?;
    info!("Jogo encerrado (status: {}). Reiniciando...", exit_status);

    Ok(())
}

async fn heartbeat_loop(
    cs_url: &str,
    machine_code: &str,
    config_path: &PathBuf,
    game_pid: SharedGamePid,
    game_status: status_listener::GameStatusChannels,
) {
    let mut failures = 0u32;

    // Dois timers na mesma task (não uma segunda task solta) — evita
    // repetir a classe de bug já achada em 28/07 (heartbeat_loop antigo
    // nunca cancelado, empilhando tasks concorrentes a cada restart do
    // jogo). `interval()` dispara imediatamente na 1ª volta por padrão;
    // consome essa 1ª volta pra manter o comportamento de "espera antes de
    // mandar" que o loop já tinha.
    let mut heartbeat_tick = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    let mut events_tick = tokio::time::interval(Duration::from_secs(GAME_EVENTS_INTERVAL_SECS));
    heartbeat_tick.tick().await;
    events_tick.tick().await;

    loop {
        tokio::select! {
            _ = events_tick.tick() => {
                let events = game_status.drain();
                if let Err(e) = api::send_game_events_batch(cs_url, machine_code, &events).await {
                    warn!("Falha ao enviar lote de game_events ({} evento(s) perdido(s) nessa leva): {}", events.len(), e);
                }
                continue;
            }
            _ = heartbeat_tick.tick() => {}
        }

        let game_state = game_status.peek_latest();
        let game_state = game_state.as_ref().map(|(code, _)| code.as_str());
        match api::send_heartbeat(cs_url, machine_code, game_state).await {
            Ok(Some(cmd)) if cmd.kind == "update_launcher" => {
                failures = 0;
                // Não pula mais só por bater o número da versão — apply_update
                // já decide por hash (`already_ready`), então republicar a
                // mesma versão com conteúdo diferente (esqueceu algo, subiu de
                // novo com o mesmo número) é pego certo. Chamar sempre também
                // garante que o ciclo completo (restart + self-check no boot)
                // sempre reporta de volta pra CS — sem isso, `pending_launcher_
                // version` ficava preso pra sempre quando a versão já batia,
                // travando qualquer outro comando atrás dele (achado real 24/08).
                info!(
                    "Update de launcher solicitado pelo backend: versão {} ({})",
                    cmd.version, cmd.url
                );
                if let Err(e) = update::apply_update(&cmd.version, &cmd.url, &cmd.sha256).await {
                    error!("Falha ao aplicar update de launcher: {}", e);
                }
            }
            Ok(Some(cmd)) if cmd.kind == "reboot" => {
                failures = 0;
                warn!("Reboot solicitado pelo backend — desligando a máquina.");
                // spawn (não status/wait) — o reboot mata este processo no
                // meio do caminho, esperar o status travaria.
                let _ = Command::new("systemctl").arg("reboot").spawn();
            }
            Ok(Some(cmd)) if cmd.kind == "restart_service" => {
                failures = 0;
                warn!("Restart do serviço solicitado pelo backend.");
                let _ = Command::new("systemctl").args(["restart", update::SERVICE_NAME]).spawn();
            }
            Ok(Some(cmd)) if cmd.kind == "update_game" => {
                failures = 0;
                info!(
                    "Update de jogo solicitado pelo backend: versão {} (sha256 {})",
                    cmd.version, cmd.sha256
                );
                // Grava o pin local — próxima chamada de `ensure_game_ready`
                // (depois que o jogo atual sair) já pega essa versão sozinha,
                // sem precisar de lógica de restart nova aqui.
                match load_config(config_path) {
                    Ok(mut fresh) => {
                        fresh.pinned_game_version = Some(cmd.version.clone());
                        fresh.pinned_game_sha256 = Some(cmd.sha256.clone());
                        // Update explícito veio de humano pelo backoffice — é o
                        // sinal de "sim, essa mudança de tela foi intencional".
                        // Próxima vez que o jogo for preparado, o layout ao
                        // vivo vira o novo `pinned_game_layout` mesmo que
                        // divergente do anterior, sem bloquear/avisar.
                        fresh.awaiting_layout_confirmation = true;
                        if let Err(e) = save_config(config_path, &fresh) {
                            error!("Falha ao salvar pin de versão do jogo: {}", e);
                        }
                    }
                    Err(e) => error!("Falha ao ler config pra gravar pin de versão do jogo: {}", e),
                }
                let pid = *game_pid.lock().unwrap();
                match pid {
                    Some(pid) => {
                        info!("Matando processo do jogo (pid {}) pra aplicar versão nova...", pid);
                        let _ = Command::new("kill").arg(pid.to_string()).status();
                    }
                    None => warn!("update_game recebido mas nenhum jogo rodando agora — aplica no próximo boot."),
                }
                // Sem isso, o CS nunca sabe que aplicamos — `pending_game_version`
                // fica preso pra sempre e é reoferecido em todo heartbeat
                // seguinte (achado real 28/07: heartbeat martelando a cada poucos
                // segundos numa máquina, CS reenviando o mesmo comando sem parar).
                api::report_game_version(cs_url, machine_code, &cmd.version, "success").await;
            }
            Ok(Some(cmd)) => {
                failures = 0;
                warn!("Heartbeat trouxe comando desconhecido: {:?}", cmd);
            }
            Ok(None) => {
                failures = 0;
            }
            Err(e) => {
                failures += 1;
                warn!("Heartbeat falhou ({}/3): {}", failures, e);
            }
        }
    }
}

/// Baixa (se preciso) e roda o jogo direto do tmpfs — `machine_variant`
/// (`vlt`/`street`) vem da sala onde a máquina foi ativada.
/// Sobe o X (usuário `game`, ver `.xinitrc`) se ainda não estiver rodando.
/// Best-effort: se falhar, o spawn do jogo adiante vai falhar de forma
/// visível no log — não vale travar o launcher por causa disso.
fn ensure_x_running() {
    if !std::path::Path::new("/tmp/.X11-unix/X0").exists() {
        // Roda como root direto (não `runuser -u game`) — achado real: sem
        // getty/sessão logind ativa na VT2 (desabilitamos o getty de propósito,
        // ver ensure de tty1/tty2), usuário comum não ganha permissão de abrir
        // o VT (`xf86OpenConsole: Permission denied`). Root sempre pode.
        info!("X não está rodando, iniciando...");
        // `startx` acha `~/.xinitrc` via $HOME — sem isso setado (systemd não
        // seta HOME pra serviços por padrão), cai no `/etc/X11/xinit/xinitrc`
        // do sistema, que não acha `.xinitrc` nenhum e abre um xterm de
        // fallback (achado real: terminal root aparecendo em vez do jogo).
        let spawned = Command::new("startx")
            .env("HOME", "/root")
            .args(["--", &format!("vt{}", GAME_VT), "-nocursor"])
            .spawn();
        if let Err(e) = spawned {
            warn!("Falha ao iniciar X: {}", e);
            return;
        }
        let mut ready = false;
        for _ in 0..20 {
            if std::path::Path::new("/tmp/.X11-unix/X0").exists() {
                info!("X pronto.");
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        if !ready {
            warn!("X não respondeu em 10s — seguindo mesmo assim.");
            return;
        }
    }
    configure_display_layout();
}

/// Detecta saídas conectadas (`xrandr --query`, campo "connected") e monta
/// o layout físico: hoje o jogo só existe em build `800x600` por tela — 1
/// saída = single screen, 2+ = a 2ª logo abaixo da 1ª (`--below`), igual o
/// setup manual testado (`xrandr --output HDMI-2 ... --output DP-2
/// --below HDMI-2`). Quando existir variante de 1 tela de verdade, a
/// resolução por contagem de tela deixa de ser fixa — hoje só cobre o caso
/// dual que já temos.
fn configure_display_layout() {
    let output = Command::new("xrandr")
        .env("DISPLAY", X_DISPLAY)
        .env("XAUTHORITY", X_AUTHORITY)
        .arg("--query")
        .output();
    let names: Vec<String> = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                (fields.len() >= 2 && fields[1] == "connected").then(|| fields[0].to_string())
            })
            .collect(),
        _ => {
            warn!("Falha ao consultar xrandr --query pra montar layout de tela");
            return;
        }
    };

    let mut cmd = Command::new("xrandr");
    cmd.env("DISPLAY", X_DISPLAY).env("XAUTHORITY", X_AUTHORITY);
    let mode_used;
    match names.as_slice() {
        [] => {
            warn!("Nenhuma saída de vídeo conectada detectada via xrandr");
            return;
        }
        [only] => {
            // 1 tela = build `single_screen_vertical`, gabinete físico monta
            // o monitor de lado — precisa girar (achado no `.xinitrc` de
            // referência do gabinete: `1920x1080 --rotate left`), diferente
            // do `800x600` sem rotação usado no caso dual.
            mode_used = SINGLE_SCREEN_MODE;
            cmd.args(["--output", only, "--mode", mode_used, "--rotate", "left", "--primary"]);
        }
        [first, second, ..] => {
            // `--rotate normal` explícito nos dois: sem isso, uma saída que
            // rodou antes em modo single (`--rotate left`) fica travada
            // nessa rotação e o xrandr recusa o mode set do dual inteiro
            // (achado real 19/08: máquina foi de single pra dual sem
            // reiniciar, `xrandr` saiu com status 1 até o reboot limpar o X).
            mode_used = GAME_SCREEN_MODE;
            cmd.args(["--output", first, "--mode", mode_used, "--rotate", "normal", "--primary"]);
            cmd.args(["--output", second, "--mode", mode_used, "--rotate", "normal", "--below", first]);
        }
    }
    match cmd.output() {
        Ok(o) if o.status.success() => info!("Layout de tela configurado ({:?}): {:?}", mode_used, names),
        Ok(o) => warn!(
            "xrandr saiu com status {} configurando {:?}: {}",
            o.status,
            names,
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(e) => warn!("Falha ao rodar xrandr: {}", e),
    }
}

/// Args fixos do jogo pra `single_screen_vertical` — achado no `.xinitrc`
/// de referência do gabinete de 1 tela: sem `--button-hub` (esse gabinete
/// não tem buttonhub físico), com `--top-header-above-video` a mais.
/// Diferente do caso dual (`settings.game_args`, configurável via
/// `VLT_GAME_ARGS`) porque hoje não existe mecanismo de variar isso por
/// layout — fica hardcoded até esse gabinete ganhar configuração própria.
const SINGLE_SCREEN_GAME_ARGS: &[&str] =
    &["--channel", "web", "--layout=stacked_dual", "--top-header-font", "--top-header-above-video"];

// Combina rgs_url (schema+host, ex: "https://191.9.124.164") + rgs_port
// (separado, número ou string) que a CS devolveu no /launcher — mesmo
// servidor que autenticou o token é quem o jogo deve usar via `--rgs-url`.
// Sem isso, cada troca de backend (ex: Contabo -> stage-gang) exigiria
// rebuildar o jogo, já que a URL ficava só fixa em `GameControl.hx`.
fn build_rgs_url(rgs_url: &str, rgs_port: &serde_json::Value) -> Option<String> {
    if rgs_url.is_empty() {
        return None;
    }
    let base = rgs_url.trim_end_matches('/');
    let port_str = match rgs_port {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    match port_str {
        Some(port) => Some(format!("{}:{}", base, port)),
        None => Some(base.to_string()),
    }
}

async fn spawn_game_and_wait(
    token: &str,
    rgs_url: Option<&str>,
    cfg: &LauncherConfig,
    config_path: &PathBuf,
    settings: &LauncherSettings,
    game_pid: SharedGamePid,
) -> Result<std::process::ExitStatus> {
    let machine_variant = &cfg.machine_variant;
    let mut screen = tui::enter_screen()?;
    let mut last_draw = std::time::Instant::now() - Duration::from_secs(1);

    // X precisa estar de pé pro xrandr enxergar as saídas conectadas — sem
    // isso `detect_machine_type()` não vê nenhuma tela e cai no fallback de
    // 1 tela sempre, mesmo em máquina dual_screen (achado real: build errada
    // sendo baixada por causa disso).
    ensure_x_running();
    let layout = hardware::video::detect_machine_type();

    if cfg.awaiting_layout_confirmation {
        // Um `update_game` explícito chegou pelo backoffice desde a última
        // vez — humano confirmou a intenção de mudança, então o que tiver
        // conectado agora vira o novo baseline, mesmo que divergente do pin
        // anterior (ex: operador corrigiu instalação de 1 pra 2 telas e já
        // subiu a build dual). Consome a janela (volta a `false`) pra não
        // aceitar mudanças espontâneas depois.
        if let Ok(mut fresh) = load_config(config_path) {
            fresh.pinned_game_layout = Some(layout.to_string());
            fresh.awaiting_layout_confirmation = false;
            if let Err(e) = save_config(config_path, &fresh) {
                warn!("Falha ao confirmar novo layout pinado: {}", e);
            } else {
                info!("Layout {:?} confirmado como novo baseline (update de jogo explícito).", layout);
            }
        }
    } else if let Some(pinned_layout) = cfg.pinned_game_layout.as_deref() {
        // Máquina já tem um layout de baseline (pin) e o que tá conectado
        // agora é outro, sem confirmação explícita — não adianta baixar
        // build nenhuma, vai dar hash mismatch lá na frente (achado real
        // 19/08: máquina pinada em dual_screen com só 1 monitor conectado,
        // ficava tentando pra sempre com erro genérico). Avisa na tela e
        // deixa o loop de retry de 30s (no chamador) reobservar sozinho —
        // assim que o monitor certo aparecer, essa checagem passa a bater.
        // Também bloqueia o sentido contrário (monitor a mais aparecendo
        // sozinho, sem update pedido) de propósito: só muda com confirmação.
        if pinned_layout != layout {
            tui::draw_lines(
                &mut screen,
                "Preparando Jogo",
                &[
                    format!("Máquina configurada para {}.", pinned_layout),
                    format!("Detectei {} agora.", layout),
                    "Confere o(s) monitor(es) conectado(s).".to_string(),
                ],
            )?;
            std::thread::sleep(Duration::from_secs(5));
            tui::leave_screen(screen)?;
            anyhow::bail!(
                "layout de tela não confere: esperado {} (pin), detectado {} — confira os monitores conectados",
                pinned_layout,
                layout
            );
        }
    } else if cfg.pinned_game_version.is_some() {
        // Máquina já pareada/pinada antes desse campo existir — sem
        // baseline de layout pra comparar. Backfill: já tá rodando de
        // verdade com esse layout agora, então assume que é o correto
        // (só roda esse ramo 1x por máquina, próximo boot já cai no `if`
        // acima). Sem isso, a checagem de mismatch nunca ativaria pra
        // frota já em produção, só pra máquina pareada depois desse fix.
        if let Ok(mut fresh) = load_config(config_path) {
            fresh.pinned_game_layout = Some(layout.to_string());
            if let Err(e) = save_config(config_path, &fresh) {
                warn!("Falha ao gravar backfill de layout pinado: {}", e);
            } else {
                info!("Layout {:?} gravado como baseline (backfill de máquina já pareada)", layout);
            }
        }
    }

    let game_args: Vec<String> = if layout == "single_screen_vertical" {
        SINGLE_SCREEN_GAME_ARGS.iter().map(|s| s.to_string()).collect()
    } else {
        settings.game_args.clone()
    };
    let pinned = cfg.pinned_game_version.as_deref().zip(cfg.pinned_game_sha256.as_deref());
    let prepare = game_runtime::ensure_game_ready(&settings.game_registry_url, GAME_TYPE, machine_variant, layout, pinned, |status| {
        // Baixa em stream chama isso por chunk — sem throttle a tela pisca
        // (redesenha centenas de vezes por segundo à toa).
        let is_final = matches!(status, game_runtime::GameStatus::AlreadyReady { .. });
        if !is_final && last_draw.elapsed() < Duration::from_millis(200) {
            return;
        }
        last_draw = std::time::Instant::now();

        let lines = match status {
            game_runtime::GameStatus::CheckingVersion => {
                vec!["Verificando versão do jogo...".to_string(), "(Esc cancela)".to_string()]
            }
            game_runtime::GameStatus::Downloading { downloaded, total } => match total {
                Some(t) if t > 0 => {
                    let pct = (downloaded as f64 / t as f64 * 100.0).min(100.0) as u32;
                    vec![
                        "Baixando jogo...".to_string(),
                        format!("{}%  ({} MB / {} MB)", pct, downloaded / 1_000_000, t / 1_000_000),
                        "(Esc cancela)".to_string(),
                    ]
                }
                _ => vec![
                    "Baixando jogo...".to_string(),
                    format!("{} MB", downloaded / 1_000_000),
                    "(Esc cancela)".to_string(),
                ],
            },
            game_runtime::GameStatus::Extracting => vec!["Descompactando jogo...".to_string()],
            game_runtime::GameStatus::AlreadyReady { version } => {
                vec![format!("Versão {} já pronta.", version)]
            }
        };
        let _ = tui::draw_lines(&mut screen, "Preparando Jogo", &lines);
    });

    // Sem isso, essa tela ficava surda a qualquer tecla — raw mode desliga
    // até o Ctrl+C virar sinal, então travava de verdade se o registry não
    // respondesse (só saía matando o processo de outro terminal).
    let ready = tokio::select! {
        result = prepare => result.context("Falha ao preparar build do jogo"),
        _ = wait_for_escape() => {
            tui::leave_screen(screen)?;
            anyhow::bail!("Preparação do jogo cancelada pelo operador (Esc)");
        }
    };

    let (bin_path, game_dir, resolved_version, resolved_sha256) = match ready {
        Ok(paths) => paths,
        Err(e) => {
            tui::draw_lines(&mut screen, "Preparando Jogo", &["Falha ao preparar jogo:".to_string(), e.to_string()])?;
            std::thread::sleep(Duration::from_secs(5));
            tui::leave_screen(screen)?;
            return Err(e);
        }
    };

    // 1º boot pós-pareamento sem pin ainda — a versão resolvida via
    // `/game/latest` vira o pin baseline no CS (e local), daí em diante
    // toda máquina fica travada numa versão específica em vez de sempre
    // pegar o que for publicado por último.
    if cfg.pinned_game_version.is_none() {
        api::report_game_version(&cfg.cs_url, &cfg.machine_code, &resolved_version, "success").await;
        if let Ok(mut fresh) = load_config(config_path) {
            fresh.pinned_game_version = Some(resolved_version.clone());
            fresh.pinned_game_sha256 = Some(resolved_sha256.clone());
            fresh.pinned_game_layout = Some(layout.to_string());
            if let Err(e) = save_config(config_path, &fresh) {
                warn!("Falha ao gravar pin local de versão do jogo: {}", e);
            }
        }
    }

    tui::draw_lines(&mut screen, "Preparando Jogo", &["Pronto! Iniciando...".to_string()])?;
    tui::leave_screen(screen)?;

    info!("Iniciando jogo: {:?} {:?}", bin_path, game_args);

    // Token via env var, não `--token` em argv — argv de qualquer processo
    // é visível a qualquer usuário local via `ps aux`/`/proc/<pid>/cmdline`,
    // env var do processo filho não (só root ou o dono do processo leem
    // `/proc/<pid>/environ`). Fix de segurança já mapeado no roadmap.
    // Jogo é gráfico (X11) — launcher roda via systemd, não herda DISPLAY de
    // sessão nenhuma. Usa o DISPLAY do próprio launcher se vier setado
    // (override), senão cai no padrão da VLT física.
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| X_DISPLAY.to_string());

    // X e launcher rodam como root os dois agora (`ensure_x_running` sobe
    // via `startx` direto, sem trocar de usuário) — cookie fica em
    // `/root/.Xauthority`, criado pelo próprio `startx`.
    let xauthority = std::env::var("XAUTHORITY").unwrap_or_else(|_| X_AUTHORITY.to_string());

    info!(
        "Config de execução — bin: {:?} | cwd: {:?} | DISPLAY={} | XAUTHORITY={} (existe: {}) | args: {:?}",
        bin_path,
        game_dir,
        display,
        xauthority,
        std::path::Path::new(&xauthority).is_file(),
        game_args
    );
    if let Ok(meta) = std::fs::metadata(&bin_path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            info!("Binário existe, {} bytes, permissões {:o}", meta.len(), meta.permissions().mode() & 0o777);
        }
    } else {
        warn!("Binário {:?} não existe no momento do spawn!", bin_path);
    }

    // stdout/stderr do jogo iam pro terminal por padrão — mas a TUI
    // (raw mode/alt screen) cobre isso, então erro de SDL/lib faltando
    // nunca sobrava pra ler depois. Grava num arquivo fixo (sobrescreve a
    // cada boot do jogo — só o último run importa pra debug).
    let game_log = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(GAME_LOG_PATH)
        .with_context(|| format!("Falha ao abrir {}", GAME_LOG_PATH))?;
    let game_log_stderr = game_log.try_clone().context("Falha ao duplicar handle do log do jogo")?;

    // X já foi garantido lá em cima (antes da detecção de layout) — aqui só
    // troca de VT pro jogo aparecer. Ensure é idempotente (checa X0 antes de
    // tentar subir de novo), sem custo repetir a chamada.
    ensure_x_running();
    let _ = Command::new("chvt").arg(GAME_VT).status();

    // O jogo (RuntimeConfig.hx::extractTokenFromArgs) só lê token de
    // `Sys.args()` — não tem leitura de env var nenhuma hoje. `GAME_TOKEN`
    // no ambiente fica de bônus (sem uso ainda, caso o jogo ganhe suporte
    // depois), mas `--token` no argv é o que faz o jogo autenticar de
    // verdade — sem isso ele roda sem token nenhum, sem dar erro visível.
    let mut cmd = Command::new(&bin_path);
    cmd.current_dir(&game_dir)
        .env("GAME_TOKEN", token)
        .env("DISPLAY", display)
        .env("XAUTHORITY", xauthority)
        .arg("--token")
        .arg(token);
    // `--rgs-url` repassa o mesmo servidor que a CS usou pra autenticar o
    // token (RuntimeConfig.hx::applyRgsUrl) — permite trocar de backend
    // (Contabo <-> stage-gang etc) sem rebuild do jogo, o build baixado do
    // registry já serve pra qualquer ambiente.
    if let Some(url) = rgs_url {
        cmd.arg("--rgs-url").arg(url);
    }
    // `--machineId` (RuntimeConfig.hx::applyMachineId) — sem isso o jogo
    // cai no hardcoded `vltMachineId = "5"` (GameControl.hx). Achado 25/08:
    // nunca foi passado pelo launcher.
    cmd.arg("--machineId").arg(&cfg.machine_code);
    let mut child = cmd
        .args(&game_args)
        .stdout(game_log)
        .stderr(game_log_stderr)
        .spawn()
        .with_context(|| format!("Falha ao iniciar jogo: {:?}", bin_path))?;

    *game_pid.lock().unwrap() = Some(child.id());
    let status = child
        .wait()
        .with_context(|| format!("Falha ao aguardar jogo: {:?}", bin_path));
    *game_pid.lock().unwrap() = None;
    let status = status?;

    info!("Jogo saiu com status {} — log completo em {}", status, GAME_LOG_PATH);

    // Jogo fechou (crash ou saída normal) — volta a tela pro menu/launcher.
    let _ = Command::new("chvt").arg(LAUNCHER_VT).status();

    Ok(status)
}
