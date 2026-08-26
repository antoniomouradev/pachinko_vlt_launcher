use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use tar::Archive;

const RELEASES_DIR: &str = "/opt/pachinko-launcher/releases";
const SYMLINK_PATH: &str = "/usr/local/bin/pachinko_vlt_launcher";
const PREVIOUS_TARGET_PATH: &str = "/opt/pachinko-launcher/previous_target";
const PENDING_MARKER_PATH: &str = "/opt/pachinko-launcher/pending_confirmation";
pub const SERVICE_NAME: &str = "pachinko-launcher";

#[derive(Serialize, Deserialize)]
struct InstalledMarker {
    sha256: String,
}

/// Baixa (se preciso), confere SHA-256, troca symlink atômico e reinicia o
/// serviço — mata o jogo em andamento de propósito (decisão: aplica na
/// hora, aceita interromper, ver BACKLOG.md "Update remoto do launcher").
/// Registry serve `.tar.gz` (mesmo padrão do `game_runtime.rs`) — o hash
/// publicado é do pacote inteiro, não do binário solto. Reusado tanto pelo
/// comando vindo do heartbeat (máquina pareada) quanto pelo botão
/// "Atualizar Launcher" do menu (máquina sem pareamento ainda).
pub async fn apply_update(version: &str, url: &str, sha256: &str) -> Result<()> {
    let release_dir = PathBuf::from(RELEASES_DIR).join(version);
    let bin_path = release_dir.join("pachinko_vlt_launcher");
    let marker_path = release_dir.join(".installed.json");

    let already_ready = std::fs::read_to_string(&marker_path)
        .ok()
        .and_then(|s| serde_json::from_str::<InstalledMarker>(&s).ok())
        .map(|m| m.sha256 == sha256)
        .unwrap_or(false)
        && bin_path.is_file();

    if !already_ready {
        std::fs::create_dir_all(&release_dir).with_context(|| format!("Falha ao criar {:?}", release_dir))?;
        download_and_extract(url, sha256, &release_dir).await?;
        std::fs::write(
            &marker_path,
            serde_json::to_string(&InstalledMarker { sha256: sha256.to_string() })?,
        )
        .context("Falha ao gravar marcador de instalação")?;
    }

    // Marca ANTES de trocar o symlink — se a versão nova crashar antes de
    // rodar `check_pending_update_or_rollback`, a rede de segurança do
    // systemd (ExecStartPre, ver service/pachinko-launcher-safety-net.sh)
    // ainda sabe que tem update pendente de confirmar.
    std::fs::write(PENDING_MARKER_PATH, version).context("Falha ao gravar marcador de update pendente")?;

    // Guarda o binário atual pra rollback ANTES de trocar o symlink.
    if let Ok(current) = std::fs::read_link(SYMLINK_PATH) {
        std::fs::write(PREVIOUS_TARGET_PATH, current.to_string_lossy().as_bytes())
            .context("Falha ao salvar binário anterior pra rollback")?;
    }

    let tmp_link = format!("{}.tmp", SYMLINK_PATH);
    let _ = std::fs::remove_file(&tmp_link);
    std::os::unix::fs::symlink(&bin_path, &tmp_link).context("Falha ao criar symlink temporário")?;
    std::fs::rename(&tmp_link, SYMLINK_PATH).context("Falha ao trocar symlink atômico")?;

    log::info!("Update {} pronto, reiniciando serviço...", version);
    // spawn (não status/wait) — o restart mata este processo no meio do
    // caminho, esperar o status travaria.
    Command::new("systemctl")
        .args(["restart", SERVICE_NAME])
        .spawn()
        .context("Falha ao chamar systemctl restart")?;
    Ok(())
}

/// Baixa o `.tar.gz` inteiro em memória (pacote do launcher é pequeno,
/// poucos MB — sem necessidade do streaming incremental que o download do
/// jogo usa), confere hash do pacote, descompacta em `release_dir`.
async fn download_and_extract(url: &str, expected_sha256: &str, release_dir: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .build()
        .context("Falha ao criar cliente HTTP")?;
    let resp = client.get(url).send().await.context("Falha ao baixar update")?;
    if !resp.status().is_success() {
        anyhow::bail!("download do update HTTP {}", resp.status());
    }
    let bytes = resp.bytes().await.context("Falha ao ler bytes do update")?;

    let actual_sha256 = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(&bytes))
    };
    if actual_sha256 != expected_sha256 {
        anyhow::bail!(
            "SHA-256 não confere (esperado {}, obtido {}) — update descartado",
            expected_sha256, actual_sha256
        );
    }

    let decoder = GzDecoder::new(Cursor::new(bytes.as_ref()));
    let mut archive = Archive::new(decoder);
    archive.unpack(release_dir).context("Falha ao descompactar update")?;

    let bin_path = release_dir.join("pachinko_vlt_launcher");
    if !bin_path.is_file() {
        anyhow::bail!("binário 'pachinko_vlt_launcher' não encontrado no pacote baixado");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path)?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(&bin_path, perms)?;
    }
    Ok(())
}

/// Roda bem cedo no boot — sem marcador, no-op (boot normal). Com marcador,
/// é a versão que acabou de ser trocada por update: confirma que subiu bem
/// ou reverte sozinha.
pub async fn check_pending_update_or_rollback(cs_url: &str, machine_code: Option<&str>) {
    let version = match std::fs::read_to_string(PENDING_MARKER_PATH) {
        Ok(v) => v.trim().to_string(),
        Err(_) => return,
    };

    log::info!("Update pendente de confirmação: versão {}. Rodando self-check...", version);
    if self_check() {
        log::info!("Self-check OK — update {} confirmado.", version);
        let _ = std::fs::remove_file(PENDING_MARKER_PATH);
        if let Some(code) = machine_code {
            report_update(cs_url, code, &version, "success").await;
        }
        return;
    }

    log::warn!("Self-check FALHOU — revertendo update {} sozinho.", version);
    revert_symlink();
    let _ = std::fs::remove_file(PENDING_MARKER_PATH);
    if let Some(code) = machine_code {
        report_update(cs_url, code, &version, "rollback_self_check").await;
    }
    let _ = Command::new("systemctl").args(["restart", SERVICE_NAME]).spawn();
}

/// Self-check mínimo: hardware legível + buttonhub responde. Não sobe X/jogo
/// aqui — só confirma que o binário novo não morre de cara (dependência
/// faltando, panic na inicialização, etc).
fn self_check() -> bool {
    if crate::hardware::collect().is_err() {
        return false;
    }
    match crate::hardware::buttonhub::connect(crate::hardware::buttonhub::DEFAULT_PORT) {
        Ok(conn) => {
            conn.close();
            true
        }
        Err(_) => false,
    }
}

fn revert_symlink() {
    let Ok(previous) = std::fs::read_to_string(PREVIOUS_TARGET_PATH) else {
        log::error!("Sem binário anterior salvo — não dá pra reverter automaticamente.");
        return;
    };
    let tmp_link = format!("{}.tmp", SYMLINK_PATH);
    let _ = std::fs::remove_file(&tmp_link);
    if std::os::unix::fs::symlink(previous.trim(), &tmp_link).is_ok() {
        let _ = std::fs::rename(&tmp_link, SYMLINK_PATH);
    }
}

async fn report_update(cs_url: &str, machine_code: &str, version: &str, status: &str) {
    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Falha ao criar cliente HTTP pra reportar update: {}", e);
            return;
        }
    };
    let url = format!("{}/machine/launcher_update_report", cs_url);
    let body = serde_json::json!({
        "machine_code": machine_code,
        "launcher_version": version,
        "status": status,
    });
    if let Err(e) = client.post(&url).json(&body).send().await {
        log::warn!("Falha ao reportar resultado do update pro backend: {}", e);
    }
}
