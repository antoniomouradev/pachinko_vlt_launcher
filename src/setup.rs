use anyhow::{Context, Result};
use dialoguer::{Input, Confirm};
use log::info;
use std::path::PathBuf;

use crate::config::{LauncherSettings, get_settings_path, save_settings, get_pairing_code_path};

#[derive(Debug)]
pub struct SetupArgs {
    pub cs_url: Option<String>,
    pub game_path: Option<String>,
    pub game_args: Option<String>,
    pub pairing_code: Option<String>,
}

pub fn run_setup(args: SetupArgs) -> Result<()> {
    println!("\n=== Configuração do Launcher Pachinko ===\n");

    let cs_url = match args.cs_url {
        Some(v) => v,
        None => Input::new()
            .with_prompt("URL do servidor CS")
            .default("https://pachinko.espindolasoftware.com.br".to_string())
            .interact_text()
            .context("Falha ao ler CS URL")?,
    };

    let game_path = match args.game_path {
        Some(v) => v,
        None => Input::new()
            .with_prompt("Caminho do jogo (ou lobby)")
            .default("/opt/pachinko/games/lobby".to_string())
            .interact_text()
            .context("Falha ao ler caminho do jogo")?,
    };

    let game_args_raw = match args.game_args {
        Some(v) => v,
        None => Input::new()
            .with_prompt("Argumentos extras do jogo (deixe vazio se nenhum)")
            .allow_empty(true)
            .default(
                "-release -- --env=prod --channel web --layout=stacked_dual --button-hub --top-header-font"
                    .to_string(),
            )
            .interact_text()
            .context("Falha ao ler argumentos do jogo")?,
    };

    let game_args: Vec<String> = game_args_raw
        .split_whitespace()
        .map(String::from)
        .collect();

    let game_registry_url: String = Input::new()
        .with_prompt("URL do game_registry_service (de onde o jogo é baixado)")
        .default("http://192.168.15.12:8090".to_string())
        .interact_text()
        .context("Falha ao ler URL do game_registry_service")?;

    let pairing_code = match args.pairing_code {
        Some(v) => Some(v),
        None => {
            let has_code = Confirm::new()
                .with_prompt("Tem um código de pareamento agora?")
                .default(false)
                .interact()
                .context("Falha ao ler confirmação")?;
            if has_code {
                let code: String = Input::new()
                    .with_prompt("Código de pareamento (8 caracteres)")
                    .interact_text()
                    .context("Falha ao ler código de pareamento")?;
                Some(code.trim().to_uppercase())
            } else {
                None
            }
        }
    };

    println!("\n--- Resumo ---");
    println!("  CS URL    : {}", cs_url);
    println!("  Jogo      : {}", game_path);
    println!("  Registry  : {}", game_registry_url);
    if !game_args.is_empty() {
        println!("  Args      : {}", game_args.join(" "));
    }
    if let Some(ref code) = pairing_code {
        println!("  Pareamento: {}", code);
    }
    println!();

    let confirmed = Confirm::new()
        .with_prompt("Salvar configuração?")
        .default(true)
        .interact()
        .context("Falha ao confirmar")?;

    if !confirmed {
        println!("Configuração cancelada.");
        return Ok(());
    }

    let settings = LauncherSettings { cs_url, game_path, game_args, game_registry_url };
    let settings_path = get_settings_path()?;
    save_settings(&settings_path, &settings)
        .context("Falha ao salvar launcher_settings.json")?;
    info!("Configuração salva em {}", settings_path.display());
    println!("\n✓ Configuração salva em {}", settings_path.display());

    if let Some(code) = pairing_code {
        write_pairing_code(&code)?;
        println!("✓ Código de pareamento salvo.");
    }

    println!("\nPróximo passo: execute o launcher para parear a máquina.");
    println!("  ./pachinko_vlt_launcher\n");

    Ok(())
}

fn write_pairing_code(code: &str) -> Result<()> {
    let path: PathBuf = get_pairing_code_path()?;
    std::fs::write(&path, format!("{}\n", code.trim().to_uppercase()))
        .context("Falha ao salvar arquivo de pairing code")
}
