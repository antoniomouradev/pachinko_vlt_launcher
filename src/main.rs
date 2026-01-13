use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use log::{info, warn, error};
use std::path::PathBuf;

mod hardware;
mod api;
mod config;
mod service;
mod error;

use hardware::HardwareInfo;
use api::register_machine;
use config::{LauncherConfig, load_config, save_config, get_config_path, derive_encryption_key, encrypt_secret, decrypt_secret};
use std::process::Command;

#[derive(Parser)]
#[command(name = "pachinko_vlt_launcher")]
#[command(about = "Launcher VLT para registro e autenticação de máquinas")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Instala launcher como serviço do sistema
    InstallService,
    /// Remove launcher do sistema
    UninstallService,
    /// Força novo registro da máquina
    Register,
    /// Verifica status do registro
    Check,
    /// Inicia o jogo com credenciais
    Launch,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::InstallService) => {
            service::install_service()
                .context("Falha ao instalar serviço")?;
            info!("Servico instalado com sucesso");
            Ok(())
        }
        Some(Commands::UninstallService) => {
            service::uninstall_service()
                .context("Falha ao remover serviço")?;
            info!("Servico removido com sucesso");
            Ok(())
        }
        Some(Commands::Register) => {
            register_machine_flow(true).await
        }
        Some(Commands::Check) => {
            check_registration_status().await
        }
        Some(Commands::Launch) => {
            let config_path = get_config_path()
                .context("Não foi possível determinar caminho de configuração")?;
            
            let config = load_config(&config_path)
                .context("Falha ao carregar configuração. Execute sem argumentos para registrar primeiro.")?;
            
            launch_game(&config)
        }
        None => {
            register_machine_flow(false).await
        }
    }
}

async fn register_machine_flow(force: bool) -> Result<()> {
    let config_path = get_config_path()
        .context("Não foi possível determinar caminho de configuração")?;

    let rgs_url = std::env::var("RGS_URL")
        .unwrap_or_else(|_| "http://localhost:43310".to_string());

    let mut config = if !force {
        load_config(&config_path).ok()
    } else {
        None
    };

    info!("Coletando informacoes de hardware...");
    let hw_info = hardware::collect()
        .context("Falha ao coletar informações de hardware")?;
    if config.is_none() || config.as_ref().unwrap().machine_id == 0 || force {
        info!("Registrando máquina no servidor RGS em {}...", rgs_url);
        let response = register_machine(&rgs_url, &hw_info).await
            .with_context(|| {
                format!(
                    "Falha ao registrar máquina no servidor RGS.\n  URL tentada: {}\n  Verifique se o servidor está rodando e acessível.",
                    rgs_url
                )
            })?;

        if response.status != "ok" {
            anyhow::bail!("Servidor retornou status: {}", response.status);
        }

        let encryption_key = derive_encryption_key(&response.hardware_fingerprint, response.machine_id)
            .context("Falha ao derivar chave de criptografia")?;
        
        let encrypted_access_token = response.access_token.as_ref()
            .map(|token| encrypt_secret(token, &encryption_key))
            .transpose()
            .context("Falha ao criptografar access_token")?;
        
        let encrypted_signing_secret = response.signing_secret.as_ref()
            .map(|secret| encrypt_secret(secret, &encryption_key))
            .transpose()
            .context("Falha ao criptografar signing_secret")?;
        
        config = Some(LauncherConfig {
            machine_id: response.machine_id,
            registration_pin: response.registration_pin.clone(),
            registered_at: chrono::Utc::now().to_rfc3339(),
            rgs_url: rgs_url.clone(),
            hardware_fingerprint: Some(response.hardware_fingerprint),
            access_token: encrypted_access_token,
            signing_secret: encrypted_signing_secret,
        });

        save_config(&config_path, config.as_ref().unwrap())
            .context("Falha ao salvar configuração")?;

        info!("Maquina registrada: machine_id={}", response.machine_id);

        if response.requires_approval {
            if let Some(pin) = response.registration_pin {
                warn!("PIN de Registro: {}", pin);
                warn!("Envie este PIN para o operador aprovar no backoffice.");
                warn!("Apos aprovacao, execute novamente o launcher.");
            }
        } else {
            info!("Maquina aprovada e pronta para uso");
            if response.access_token.is_some() && response.signing_secret.is_some() {
                info!("Tokens recebidos e salvos criptografados");
            }
        }
    } else {
        let mut cfg = config.unwrap();
        info!("Machine ID: {}", cfg.machine_id);
        info!("Registrado em: {}", cfg.registered_at);
        
        info!("Verificando status da maquina no servidor RGS...");
        let response = register_machine(&rgs_url, &hw_info).await
            .with_context(|| {
                format!(
                    "Falha ao verificar status da máquina no servidor RGS.\n  URL tentada: {}\n  Verifique se o servidor está rodando e acessível.",
                    rgs_url
                )
            })?;
        
        if response.status == "ok" {
            if response.access_token.is_some() && response.signing_secret.is_some() {
                let hardware_fp = cfg.hardware_fingerprint.as_ref()
                    .unwrap_or(&response.hardware_fingerprint);
                
                let encryption_key = derive_encryption_key(hardware_fp, cfg.machine_id)
                    .context("Falha ao derivar chave de criptografia")?;
                
                let encrypted_access_token = response.access_token.as_ref()
                    .map(|token| encrypt_secret(token, &encryption_key))
                    .transpose()
                    .context("Falha ao criptografar access_token")?;
                
                let encrypted_signing_secret = response.signing_secret.as_ref()
                    .map(|secret| encrypt_secret(secret, &encryption_key))
                    .transpose()
                    .context("Falha ao criptografar signing_secret")?;
                
                cfg.access_token = encrypted_access_token;
                cfg.signing_secret = encrypted_signing_secret;
                save_config(&config_path, &cfg)
                    .context("Falha ao salvar tokens atualizados")?;
                
                info!("Tokens atualizados");
            }
            
            if cfg.registration_pin.is_some() && !response.requires_approval {
                cfg.registration_pin = None;
                save_config(&config_path, &cfg)
                    .context("Falha ao atualizar configuração")?;
                info!("Maquina aprovada");
            }
            
            if cfg.registration_pin.is_some() {
                warn!("Maquina aguardando aprovacao. Execute register para verificar novamente.");
            } else if cfg.access_token.is_some() && cfg.signing_secret.is_some() {
                info!("Maquina ja registrada e aprovada. Pronta para iniciar jogo.");
            } else {
                info!("Maquina ja registrada.");
            }
        }
    }

    Ok(())
}

fn launch_game(config: &LauncherConfig) -> Result<()> {
    let hardware_fp = config.hardware_fingerprint.as_ref()
        .ok_or_else(|| anyhow::anyhow!("hardware_fingerprint não encontrado na configuração"))?;
    
    let encryption_key = derive_encryption_key(hardware_fp, config.machine_id)
        .context("Falha ao derivar chave de criptografia")?;
    
    let access_token = config.access_token.as_ref()
        .map(|enc| decrypt_secret(enc, &encryption_key))
        .transpose()
        .context("Falha ao descriptografar access_token")?;
    
    let signing_secret = config.signing_secret.as_ref()
        .map(|enc| decrypt_secret(enc, &encryption_key))
        .transpose()
        .context("Falha ao descriptografar signing_secret")?;
    
    if access_token.is_none() || signing_secret.is_none() {
        anyhow::bail!("Tokens não encontrados. Execute register para obter tokens.");
    }
    
    let game_path = std::env::var("VLT_GAME_PATH")
        .unwrap_or_else(|_| "./pachinko_game".to_string());
    
    info!("Iniciando jogo: {}", game_path);
    
    let mut cmd = Command::new(&game_path);
    
    cmd.arg("--vlt_machine_id").arg(config.machine_id.to_string());
    cmd.arg("--vlt_rgs_url").arg(&config.rgs_url);
    
    if let Some(ref token) = access_token {
        cmd.arg("--vlt_access_token").arg(token);
    }
    
    if let Some(ref secret) = signing_secret {
        cmd.arg("--vlt_signing_secret").arg(secret);
    }
    
    cmd.spawn()
        .with_context(|| format!("Falha ao iniciar jogo: {}", game_path))?;
    
    info!("Jogo iniciado com sucesso");
    Ok(())
}

async fn check_registration_status() -> Result<()> {
    let config_path = get_config_path()?;

    match load_config(&config_path) {
        Ok(config) => {
            println!("Status: Registrado");
            println!("Machine ID: {}", config.machine_id);
            println!("Registrado em: {}", config.registered_at);
            if let Some(pin) = config.registration_pin {
                println!("PIN: {} (aguardando aprovação)", pin);
            } else {
                println!("Status: Aprovado");
            }
            Ok(())
        }
        Err(_) => {
            println!("Status: Não registrado");
            println!("Execute sem argumentos para registrar.");
            Ok(())
        }
    }
}
