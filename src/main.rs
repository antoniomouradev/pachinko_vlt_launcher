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
mod service;
mod error;
mod setup;
mod tui;

use config::{LauncherConfig, LauncherSettings, load_config, save_config, delete_config, get_config_path, get_pairing_code_path, get_settings_path, load_settings};

const HEARTBEAT_INTERVAL_SECS: u64 = 30;
const PAIRING_POLL_INTERVAL_SECS: u64 = 10;

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

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

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
        cs_url: std::env::var("CS_URL").unwrap_or_else(|_| "http://localhost:8888".to_string()),
        game_path: std::env::var("VLT_GAME_PATH").unwrap_or_else(|_| "./pachinko_game".to_string()),
        game_args: std::env::var("VLT_GAME_ARGS")
            .unwrap_or_default()
            .split_whitespace()
            .map(String::from)
            .collect(),
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
                        info!("Jogo encerrado. Reiniciando...");
                    }
                    Err(e) if e.to_string().contains("401") || e.to_string().contains("403") => {
                        warn!("Credenciais inválidas ({}). Limpando config e aguardando novo pareamento.", e);
                        delete_config(&config_path);
                        wait_for_pairing(&cs_url, &fingerprint, &config_path, &settings).await?;
                    }
                    Err(e) => {
                        error!("Erro ao obter token: {}. Tentando novamente em {}s...", e, HEARTBEAT_INTERVAL_SECS);
                        sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
                    }
                }
            }
            None => {
                // Máquina crua (sem config ainda) — mostra o menu inicial em vez de
                // ir direto pro pareamento. "Configurar Máquina" cai no fluxo atual
                // de pareamento; device flow de verdade substitui isso depois.
                match tui::run_menu()? {
                    Some(tui::MenuChoice::ConfigureMachine) => {
                        wait_for_pairing(&cs_url, &fingerprint, &config_path, &settings).await?;
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

    let exit_status = spawn_game_and_wait(&token_resp.token, settings)?;
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

        let mut lines = vec!["Ping contínuo — Esc/q para sair".to_string(), String::new()];
        lines.extend(history.iter().cloned());
        if let Err(e) = tui::draw_lines(&mut screen, "Conexão", &lines) {
            break Err(e);
        }

        match tui::poll_key(Duration::from_secs(1)) {
            Ok(Some(KeyCode::Esc)) | Ok(Some(KeyCode::Char('q'))) => break Ok(()),
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

                        let exit_status = spawn_game_and_wait(&resp.token, settings)?;
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

fn spawn_game_and_wait(token: &str, settings: &LauncherSettings) -> Result<std::process::ExitStatus> {
    info!("Iniciando jogo: {}", settings.game_path);

    let status = Command::new(&settings.game_path)
        .arg("--token")
        .arg(token)
        .args(&settings.game_args)
        .spawn()
        .with_context(|| format!("Falha ao iniciar jogo: {}", settings.game_path))?
        .wait()
        .with_context(|| format!("Falha ao aguardar jogo: {}", settings.game_path))?;

    Ok(status)
}
