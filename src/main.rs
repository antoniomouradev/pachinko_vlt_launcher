use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::event::KeyCode;
use log::{info, warn, error};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

mod hardware;
mod api;
mod config;
mod game_runtime;
mod service;
mod error;
mod setup;
mod tui;

use config::{LauncherConfig, LauncherSettings, load_config, save_config, delete_config, get_config_path, get_pairing_code_path, get_settings_path, load_settings};

const HEARTBEAT_INTERVAL_SECS: u64 = 30;
#[allow(dead_code)] // usado só no fluxo antigo (wait_for_pairing), ver comentário lá
const PAIRING_POLL_INTERVAL_SECS: u64 = 10;
const DEVICE_FLOW_RETRY_SECS: u64 = 10;
const GAME_RESTART_DELAY_SECS: u64 = 3;

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
        cs_url: std::env::var("CS_URL").unwrap_or_else(|_| "https://pachinko.espindolasoftware.com.br".to_string()),
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
            .unwrap_or_else(|_| "http://192.168.15.12:8090".to_string()),
    }
}

async fn run() -> Result<()> {
    let settings = load_runtime_settings();
    let cs_url = settings.cs_url.clone();

    let hw_info = hardware::collect()
        .context("Falha ao coletar informações de hardware")?;
    let fingerprint = hardware::compute_fingerprint(&hw_info);
    info!("Hardware fingerprint: {}", &fingerprint[..20]);

    let config_path = get_config_path()
        .context("Não foi possível determinar caminho de configuração")?;

    loop {
        let config = load_config(&config_path).ok();

        match config {
            Some(cfg) => {
                match try_get_token_and_run(&cs_url, &cfg, &fingerprint, &config_path, &settings).await {
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
                        run_device_flow(&cs_url, &fingerprint, &config_path, &settings).await?;
                    }
                    Err(e) => {
                        error!("Erro ao obter token: {}. Tentando novamente em {}s...", e, HEARTBEAT_INTERVAL_SECS);
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
                        run_device_flow(&cs_url, &fingerprint, &config_path, &settings).await?;
                    }
                    Some(tui::MenuChoice::TestMachine) => {
                        run_test_menu_loop().await?;
                    }
                    Some(tui::MenuChoice::Shutdown) => {
                        info!("Desligado pelo menu.");
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
    config_path: &PathBuf,
    settings: &LauncherSettings,
) -> Result<()> {
    info!("Obtendo token para máquina {}...", config.machine_code);
    let token_resp = api::get_token(cs_url, &config.machine_code, fingerprint).await?;

    info!("Token obtido. Iniciando jogo...");
    let heartbeat_cs_url = cs_url.to_string();
    let machine_code = config.machine_code.clone();
    let heartbeat_config_path = config_path.clone();

    tokio::spawn(async move {
        heartbeat_loop(&heartbeat_cs_url, &machine_code, &heartbeat_config_path).await;
    });

    let exit_status = spawn_game_and_wait(&token_resp.token, &config.machine_variant, settings).await?;
    info!("Jogo encerrado (status: {}). Reiniciando...", exit_status);

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

        let mut lines = vec!["Ping contínuo — Esc para sair".to_string(), String::new()];
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
                        };
                        save_config(config_path, &cfg)
                            .context("Falha ao salvar configuração após pareamento")?;
                        let _ = std::fs::remove_file(&pairing_file);
                        let _ = std::env::remove_var("PAIRING_CODE");
                        info!("Máquina pareada: {}", resp.machine_code);

                        tokio::spawn({
                            let cs = cs_url.to_string();
                            let mc = resp.machine_code.clone();
                            let cp = config_path.clone();
                            async move { heartbeat_loop(&cs, &mc, &cp).await; }
                        });

                        let exit_status = spawn_game_and_wait(&resp.token, &cfg.machine_variant, settings).await?;
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
    config_path: &PathBuf,
    settings: &LauncherSettings,
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
        match api::activate_device(cs_url, &machine_code, fingerprint, machine_type, &id_island, position).await {
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

    finish_device_flow(cs_url, fingerprint, config_path, settings, approved, machine_variant).await
}

async fn finish_device_flow(
    cs_url: &str,
    fingerprint: &str,
    config_path: &PathBuf,
    settings: &LauncherSettings,
    approved: api::DeviceActivateResponse,
    machine_variant: String,
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
    };
    save_config(config_path, &cfg).context("Falha ao salvar configuração após aprovação")?;
    info!("Máquina ativada via device flow: {}", machine_code);

    tokio::spawn({
        let cs = cs_url.to_string();
        let mc = machine_code.clone();
        let cp = config_path.clone();
        async move {
            heartbeat_loop(&cs, &mc, &cp).await;
        }
    });

    let exit_status = spawn_game_and_wait(&token, &cfg.machine_variant, settings).await?;
    info!("Jogo encerrado (status: {}). Reiniciando...", exit_status);

    Ok(())
}

async fn heartbeat_loop(cs_url: &str, machine_code: &str, _config_path: &PathBuf) {
    let mut failures = 0u32;
    loop {
        sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
        match api::send_heartbeat(cs_url, machine_code).await {
            Ok(()) => {
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
async fn spawn_game_and_wait(
    token: &str,
    machine_variant: &str,
    settings: &LauncherSettings,
) -> Result<std::process::ExitStatus> {
    let mut screen = tui::enter_screen()?;
    let mut last_draw = std::time::Instant::now() - Duration::from_secs(1);

    let prepare = game_runtime::ensure_game_ready(&settings.game_registry_url, GAME_TYPE, machine_variant, |status| {
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

    let (bin_path, game_dir) = match ready {
        Ok(paths) => paths,
        Err(e) => {
            tui::draw_lines(&mut screen, "Preparando Jogo", &["Falha ao preparar jogo:".to_string(), e.to_string()])?;
            std::thread::sleep(Duration::from_secs(5));
            tui::leave_screen(screen)?;
            return Err(e);
        }
    };

    tui::draw_lines(&mut screen, "Preparando Jogo", &["Pronto! Iniciando...".to_string()])?;
    tui::leave_screen(screen)?;

    info!("Iniciando jogo: {:?} {:?}", bin_path, settings.game_args);

    // Token via env var, não `--token` em argv — argv de qualquer processo
    // é visível a qualquer usuário local via `ps aux`/`/proc/<pid>/cmdline`,
    // env var do processo filho não (só root ou o dono do processo leem
    // `/proc/<pid>/environ`). Fix de segurança já mapeado no roadmap.
    // Jogo é gráfico (X11) — launcher roda via systemd, não herda DISPLAY de
    // sessão nenhuma. Usa o DISPLAY do próprio launcher se vier setado
    // (override), senão cai no padrão da VLT física.
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0.0".to_string());

    // Launcher roda como root (precisa pro tmpfs/dmidecode), mas o X server
    // normalmente é iniciado por outro usuário — sem XAUTHORITY apontando
    // pro cookie de quem iniciou o X, root não tem permissão de conectar
    // (SDL não acha "video device" mesmo com X de pé). Path padrão do Xauth
    // do usuário `game` (mesmo da sessão gráfica, ver `.xinitrc`).
    let xauthority = std::env::var("XAUTHORITY").unwrap_or_else(|_| "/home/game/.Xauthority".to_string());

    info!(
        "Config de execução — bin: {:?} | cwd: {:?} | DISPLAY={} | XAUTHORITY={} (existe: {}) | args: {:?}",
        bin_path,
        game_dir,
        display,
        xauthority,
        std::path::Path::new(&xauthority).is_file(),
        settings.game_args
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

    // O jogo (RuntimeConfig.hx::extractTokenFromArgs) só lê token de
    // `Sys.args()` — não tem leitura de env var nenhuma hoje. `GAME_TOKEN`
    // no ambiente fica de bônus (sem uso ainda, caso o jogo ganhe suporte
    // depois), mas `--token` no argv é o que faz o jogo autenticar de
    // verdade — sem isso ele roda sem token nenhum, sem dar erro visível.
    let status = Command::new(&bin_path)
        .current_dir(&game_dir)
        .env("GAME_TOKEN", token)
        .env("DISPLAY", display)
        .env("XAUTHORITY", xauthority)
        .arg("--token")
        .arg(token)
        .args(&settings.game_args)
        .stdout(game_log)
        .stderr(game_log_stderr)
        .spawn()
        .with_context(|| format!("Falha ao iniciar jogo: {:?}", bin_path))?
        .wait()
        .with_context(|| format!("Falha ao aguardar jogo: {:?}", bin_path))?;

    info!("Jogo saiu com status {} — log completo em {}", status, GAME_LOG_PATH);
    Ok(status)
}
