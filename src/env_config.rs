//! Ambiente de deploy escolhido em tempo de compilação via `LAUNCHER_ENV`
//! (ex: `LAUNCHER_ENV=street cargo build --release`). Hoje bate 1:1 com a
//! variante física da máquina (`vlt`/`street`, ver `config::LauncherConfig
//! ::machine_variant`) — `street` sempre aponta pro stage-gang, `vlt`
//! sempre aponta pro Contabo, `pb` sempre aponta pro pb-server (Paraiba/
//! Diamond). Sem a env var, cai no `vlt`/Contabo (produção "Lado A",
//! default de sempre). Ambiente novo que não seja `vlt`/`street`/`pb`
//! = 1 linha em cada função abaixo.
//!
//! `local` (21/09) — pra testar integração Zylott sem servidor dedicado
//! ainda: CS e zylott_service rodando no Mac de quem tá testando (LAN
//! `192.168.15.6`, docker local), mas o game_registry continua sendo o do
//! stage-gang — não precisa manter build do jogo local, `0002` já baixa
//! de lá hoje (`street`). Se o IP do Mac mudar (DHCP), recompilar.

pub fn default_cs_url() -> &'static str {
    match option_env!("LAUNCHER_ENV") {
        Some("street") => "https://p3-sp-teste.espindola.software",
        Some("pb") => "https://p3-pb-prod.espindola.software",
        Some("local") => "http://192.168.15.6:8888",
        _ => "https://pachinko.espindolasoftware.com.br",
    }
}

pub fn default_game_registry_url() -> &'static str {
    match option_env!("LAUNCHER_ENV") {
        Some("street") => "https://p3-sp-teste.espindola.software:8090",
        Some("pb") => "https://p3-pb-prod.espindola.software/registry",
        Some("local") => "https://p3-sp-teste.espindola.software:8090",
        _ => "https://pachinko.espindolasoftware.com.br:8090",
    }
}

/// Volume padrão setado no boot (`hardware::audio::set_volume`) — Morango
/// (`street`) reclamou de 70% alto demais, pedido pra sair fixo em 50% só
/// nessa variante (achado 03/09, ver memória de sessão).
pub fn default_volume_percent() -> u8 {
    match option_env!("LAUNCHER_ENV") {
        Some("street") => 50,
        _ => 70,
    }
}

/// URL do `zylott_service` (porta 43500) — launcher fala direto com ele
/// pra `install`/`authentic`/`keepalive` (controle de terminal, não
/// dinheiro — ver `zylott_service/NOTES_INTEGRACAO.md`). Só existe rodando
/// em `local` por enquanto (nenhum ambiente remoto tem o serviço
/// deployado ainda). Retorna `None` = feature desligada, launcher não
/// tenta falar com Zylott.
pub fn default_zylott_service_url() -> Option<&'static str> {
    match option_env!("LAUNCHER_ENV") {
        Some("local") => Some("http://192.168.15.6:43500"),
        _ => None,
    }
}
