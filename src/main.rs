use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
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
                wait_for_pairing(&cs_url, &fingerprint, &config_path, &settings).await?;
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
