//! Self-heal de rede — chamado quando o launcher não consegue falar com o
//! CS. Achado real (24/07): nome de interface muda por motherboard/slot
//! PCI (`eno1` na máquina de referência, `enp1s0`/`enp2s0` numa clonada em
//! hardware diferente) — a config estática do `ifupdown`
//! (`/etc/network/interfaces`, vem hardcoded na imagem golden) nunca ativa
//! DHCP numa interface com nome diferente do gravado. Em vez de depender
//! de reboot pra pegar rede, o launcher confere e conserta sozinho aqui:
//! interface com hardware real mas sem IPv4 configurado → sobe + dhclient.
//! Precisa de `isc-dhcp-client` instalado (pacote `dhclient`).

use std::fs;
use std::process::Command;

pub fn try_self_heal() {
    let Ok(entries) = fs::read_dir("/sys/class/net") else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "lo" {
            continue;
        }
        if !entry.path().join("device").exists() {
            continue; // sem hardware real por trás (docker0, veth, etc)
        }
        if has_ipv4(&name) {
            continue; // já tem IP, não é essa que tá com problema
        }
        log::warn!("Self-heal de rede: interface {} sem IPv4, tentando subir + dhclient...", name);
        let _ = Command::new("ip").args(["link", "set", &name, "up"]).status();
        match Command::new("dhclient").arg(&name).status() {
            Ok(s) if s.success() => log::info!("Self-heal de rede: dhclient em {} rodou OK.", name),
            Ok(s) => log::warn!("Self-heal de rede: dhclient em {} saiu com status {}.", name, s),
            Err(e) => log::warn!("Self-heal de rede: falha ao rodar dhclient em {}: {}", name, e),
        }
    }
}

fn has_ipv4(iface: &str) -> bool {
    Command::new("ip")
        .args(["-4", "-br", "addr", "show", iface])
        .output()
        .ok()
        .map(|o| {
            // Sem IP: "enp1s0    DOWN" (2 campos). Com IP: "enp1s0  UP  192.168.1.5/24" (3+).
            String::from_utf8_lossy(&o.stdout).split_whitespace().count() >= 3
        })
        .unwrap_or(false)
}
