//! Self-heal de SSH — garante que o sshd está de pé no boot. Achado real
//! (11/09, máquina 8301 do Diamond): CS/heartbeat funcionando (launcher
//! conectado normal), mas porta 22 recusando conexão — sshd não subiu nesse
//! boot, sem outro jeito remoto de diagnosticar a máquina. Roda uma vez no
//! início de `run()`, igual o network_selfheal.
use std::process::Command;

pub fn ensure_sshd_running() {
    let active = Command::new("systemctl")
        .args(["is-active", "--quiet", "ssh"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if active {
        return;
    }

    log::warn!("Self-heal de SSH: serviço 'ssh' não está ativo, tentando iniciar...");
    match Command::new("systemctl").args(["start", "ssh"]).status() {
        Ok(s) if s.success() => log::info!("Self-heal de SSH: serviço 'ssh' iniciado OK."),
        Ok(s) => log::warn!("Self-heal de SSH: 'systemctl start ssh' saiu com status {}.", s),
        Err(e) => log::warn!("Self-heal de SSH: falha ao rodar 'systemctl start ssh': {}", e),
    }
}
