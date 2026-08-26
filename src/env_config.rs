//! Ambiente de deploy escolhido em tempo de compilação via `LAUNCHER_ENV`
//! (ex: `LAUNCHER_ENV=street cargo build --release`). Hoje bate 1:1 com a
//! variante física da máquina (`vlt`/`street`, ver `config::LauncherConfig
//! ::machine_variant`) — `street` sempre aponta pro stage-gang, `vlt`
//! sempre aponta pro Contabo, `pb` sempre aponta pro pb-server (Paraiba/
//! Diamond). Sem a env var, cai no `vlt`/Contabo (produção "Lado A",
//! default de sempre). Ambiente novo que não seja `vlt`/`street`/`pb`
//! = 1 linha em cada função abaixo.

pub fn default_cs_url() -> &'static str {
    match option_env!("LAUNCHER_ENV") {
        Some("street") => "https://p3-sp-teste.espindola.software",
        Some("pb") => "https://p3-pb-prod.espindola.software",
        _ => "https://pachinko.espindolasoftware.com.br",
    }
}

pub fn default_game_registry_url() -> &'static str {
    match option_env!("LAUNCHER_ENV") {
        Some("street") => "https://p3-sp-teste.espindola.software:8090",
        Some("pb") => "https://p3-pb-prod.espindola.software/registry",
        _ => "https://pachinko.espindolasoftware.com.br:8090",
    }
}
