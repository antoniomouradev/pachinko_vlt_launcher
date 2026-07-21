//! Baixa e roda o jogo em RAM — nunca disco. Baixa em memória, confere
//! sha256 contra o `game_registry_service`, descompacta direto num tmpfs.
//! Recompra só acontece se a versão publicada mudou ou o tmpfs tá vazio
//! (boot novo) — reinício de processo dentro do mesmo boot reaproveita o
//! que já tá extraído (`.installed.json` guarda o hash da versão instalada).

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tar::Archive;

use crate::api;

const TMPFS_ROOT: &str = "/run/pachinko-game";
const TMPFS_SIZE: &str = "1024M";

/// Nome do binário dentro do pacote, por game_type. Só `pachinko3` por
/// enquanto (decisão do usuário) — cresce quando outros jogos entrarem
/// nesse fluxo de download.
fn binary_name(game_type: &str) -> Result<&'static str> {
    match game_type {
        "pachinko3" => Ok("PachinkoGameBingo"),
        other => anyhow::bail!("binário desconhecido pro game_type '{}'", other),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct InstalledMarker {
    sha256: String,
    version: String,
}

/// Fase atual de `ensure_game_ready` — pra quem chama mostrar status na
/// tela (TUI) sem `game_runtime` precisar saber nada de desenho de tela.
#[derive(Debug, Clone, Copy)]
pub enum GameStatus<'a> {
    CheckingVersion,
    /// `total` é `None` se o servidor não mandou `Content-Length`.
    Downloading { downloaded: u64, total: Option<u64> },
    Extracting,
    AlreadyReady { version: &'a str },
}

fn ensure_tmpfs_mounted() -> Result<()> {
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let existing_line = mounts.lines().find(|line| line.split_whitespace().nth(1) == Some(TMPFS_ROOT));
    if let Some(line) = existing_line {
        info!("tmpfs já montado em {}: {}", TMPFS_ROOT, line);
        // `noexec` no meio das opções travaria o exec do binário sem erro
        // nenhum na hora do `mount` — só quando fosse rodar. Loga alto pra
        // não passar batido se algum dia entrar noutro fluxo de montagem.
        if line.split_whitespace().nth(3).is_some_and(|opts| opts.split(',').any(|o| o == "noexec")) {
            warn!("tmpfs {} está montado com `noexec` — binário do jogo não vai conseguir rodar de lá!", TMPFS_ROOT);
        }
        return Ok(());
    }

    fs::create_dir_all(TMPFS_ROOT).context("Falha ao criar diretório do tmpfs")?;
    let output = std::process::Command::new("mount")
        .args(["-t", "tmpfs", "-o", &format!("size={}", TMPFS_SIZE), "tmpfs", TMPFS_ROOT])
        .output()
        .context("Falha ao executar mount (precisa de root)")?;
    if !output.status.success() {
        anyhow::bail!(
            "mount tmpfs em {} falhou (status {}): {}",
            TMPFS_ROOT,
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mounts_after = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mount_line = mounts_after.lines().find(|line| line.split_whitespace().nth(1) == Some(TMPFS_ROOT));
    info!("tmpfs montado em {} ({}): {:?}", TMPFS_ROOT, TMPFS_SIZE, mount_line);
    Ok(())
}

/// Garante que o binário do jogo está pronto em tmpfs, baixando+extraindo
/// só se necessário. Retorna `(caminho_do_binário, diretório_base)` — os
/// assets do jogo são relativos ao diretório base, precisa rodar com esse
/// `cwd`.
pub async fn ensure_game_ready(
    registry_url: &str,
    game_type: &str,
    variant: &str,
    mut on_status: impl FnMut(GameStatus),
) -> Result<(PathBuf, PathBuf)> {
    ensure_tmpfs_mounted()?;

    let bin_name = binary_name(game_type)?;
    let game_dir = Path::new(TMPFS_ROOT).join(game_type).join(variant);
    let marker_path = game_dir.join(".installed.json");

    on_status(GameStatus::CheckingVersion);
    let latest = api::get_latest_build(registry_url, game_type, variant)
        .await
        .context("Falha ao consultar game_registry_service")?;

    let already_installed = fs::read_to_string(&marker_path)
        .ok()
        .and_then(|s| serde_json::from_str::<InstalledMarker>(&s).ok())
        .map(|m| m.sha256 == latest.sha256)
        .unwrap_or(false);

    if already_installed {
        info!(
            "Build {}/{} v{} já em tmpfs (hash confere) — pulando download",
            game_type, variant, latest.version
        );
        on_status(GameStatus::AlreadyReady { version: &latest.version });
        return Ok((game_dir.join(bin_name), game_dir));
    }

    info!(
        "Baixando build {}/{} v{} ({} bytes)...",
        game_type, variant, latest.version, latest.size_bytes
    );
    let bytes = api::download_bytes_with_progress(&latest.download_url, |downloaded, total| {
        on_status(GameStatus::Downloading { downloaded, total });
    })
    .await
    .context("Falha ao baixar build do jogo")?;

    on_status(GameStatus::Extracting);

    let actual_sha256 = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    };
    if actual_sha256 != latest.sha256 {
        anyhow::bail!(
            "hash não confere pra {}/{} v{} (esperado {}, obtido {}) — build corrompida ou download incompleto",
            game_type, variant, latest.version, latest.sha256, actual_sha256
        );
    }

    // Extração é a única escrita em disco de verdade — e é em tmpfs (RAM),
    // não no cartão/SSD da máquina.
    if game_dir.exists() {
        fs::remove_dir_all(&game_dir).context("Falha ao limpar instalação anterior em tmpfs")?;
    }
    fs::create_dir_all(&game_dir).context("Falha ao criar diretório do jogo em tmpfs")?;

    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(decoder);

    // Descompacta entrada por entrada (em vez de `archive.unpack()` de uma
    // vez) pra logar cada arquivo e garantir +x em tudo — não só no binário
    // principal. Native libs (`.ndll`, `.so`) carregadas via dlopen podem
    // precisar do bit de execução, e o pacote de origem nem sempre traz
    // isso (visto acontecer: `lime.ndll` chegou `rw-r--r--` no tarball).
    let mut extracted_count: u32 = 0;
    let mut extracted_bytes: u64 = 0;
    for entry in archive.entries().context("Falha ao ler entradas do pacote")? {
        let mut entry = entry.context("Entrada inválida no pacote do jogo")?;
        let rel_path = entry.path().context("Path inválido numa entrada do pacote")?.into_owned();
        let entry_size = entry.header().size().unwrap_or(0);
        let original_mode = entry.header().mode().unwrap_or(0o644);
        let is_dir = entry.header().entry_type().is_dir();

        entry
            .unpack_in(&game_dir)
            .with_context(|| format!("Falha ao extrair {:?} do pacote do jogo", rel_path))?;

        let dest_path = game_dir.join(&rel_path);
        if !is_dir && dest_path.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // OR com 0o755 em vez de sobrescrever — preserva bits extras
                // que já existiam (setuid/etc não existe aqui, é só defensivo)
                // e garante leitura+execução mesmo se o original não tinha.
                let new_mode = original_mode | 0o755;
                if let Err(e) = fs::set_permissions(&dest_path, fs::Permissions::from_mode(new_mode)) {
                    warn!("Falha ao ajustar permissão de {:?}: {}", dest_path, e);
                }
            }
            debug!("Extraído: {:?} ({} bytes, modo original {:o})", rel_path, entry_size, original_mode);
        }

        extracted_count += 1;
        extracted_bytes += entry_size;
    }
    info!(
        "Descompactação completa: {} entradas, {} bytes totais em {:?}",
        extracted_count, extracted_bytes, game_dir
    );

    let bin_path = game_dir.join(bin_name);
    if !bin_path.is_file() {
        anyhow::bail!("Binário do jogo não encontrado após extrair: {:?} (pacote não tem esse arquivo?)", bin_path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::metadata(&bin_path)?.permissions();
        info!("Binário do jogo em {:?}, permissões {:o}", bin_path, perms.mode() & 0o777);
    }

    fs::write(
        &marker_path,
        serde_json::to_string(&InstalledMarker { sha256: latest.sha256.clone(), version: latest.version.clone() })
            .context("Falha ao serializar marcador de instalação")?,
    )
    .context("Falha ao escrever marcador de instalação")?;

    info!("Build {}/{} v{} pronta em {:?}", game_type, variant, latest.version, game_dir);
    Ok((bin_path, game_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_name_known_game_type() {
        assert_eq!(binary_name("pachinko3").unwrap(), "PachinkoGameBingo");
    }

    #[test]
    fn binary_name_unknown_game_type_is_error() {
        assert!(binary_name("not_a_real_game").is_err());
    }
}
