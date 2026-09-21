# Changelog

Formato livre, ordem cronológica (mais recente primeiro). Cobre o launcher
(`pachinko_vlt_launcher`); mudanças no jogo/cliente ficam no changelog do
repositório do jogo.

## 0.2.25 (2026-08-25)

- Domínio do stage-gang trocou de `pachinko-teste.espindola.software` pra
  `p3-sp-teste.espindola.software` — atualizado o default embutido
  (`env_config.rs`, variant `street`). DNS do domínio antigo já parou de
  resolver na hora da troca (achado 25/08) — cert LE novo emitido, nginx e
  `game_registry_service` do stage-gang já reapontados também.

## 0.2.24 (2026-08-25)

- `--machineId` passado pro jogo no spawn (`cfg.machine_code`) — faltava,
  achado 25/08. Sem isso o jogo caía no hardcoded `vltMachineId = "5"`
  (`GameControl.hx`). Jogo já lia `--machineId`/`--machine-id`/
  `--machine_id` desde antes (`RuntimeConfig.hx::applyMachineId`), só o
  launcher nunca mandava.

## 0.2.23 (2026-08-25)

- Removida a CA privada interna (`tls.rs`, `certs/pachinko_internal_ca.pem`)
  embutida no binário desde a 0.2.19 — existia só pra permitir TLS de
  verdade contra stage-gang, que na época só tinha IP fixo (sem domínio
  público, cert não emitido pra IP puro). Desde 25/08 stage-gang tem
  domínio real (`pachinko-teste.espindola.software`) com Let's Encrypt
  real, então a CA extra não é mais necessária — todos os clientes HTTP
  voltam a validar só com as CAs públicas do sistema.
- URLs padrão (`CS_URL`/`GAME_REGISTRY_URL`) agora são escolhidas em
  **tempo de compilação** por ambiente, via `LAUNCHER_ENV` (novo módulo
  `env_config.rs`) — bate 1:1 com a variante física da máquina: `cargo
  build --release` (ou `LAUNCHER_ENV=vlt`) sai com default Contabo,
  `LAUNCHER_ENV=street cargo build --release` sai com default
  stage-gang. Motivo: vamos ter mais ambientes com o tempo, e antes só
  existia 1 default hardcoded (Contabo) em 3 lugares (`main.rs`,
  `config/mod.rs`, `service/linux.rs`) — agora é 1 fonte de verdade,
  ambiente novo (fora `vlt`/`street`) = 1 linha em cada função de
  `env_config.rs`.
  Também tirado o hardcode de `CS_URL`/`GAME_REGISTRY_URL` do
  `service/pachinko-launcher.service` estático (usado pelo `build.sh` /
  `install_service.sh`) — sobrescrevia via `Environment=` o default já
  certo embutido no binário, fazendo o `LAUNCHER_ENV` de build não ter
  efeito nesse caminho de instalação.

## 0.2.22 (2026-08-24)

- Removida a migração automática de servidor introduzida na 0.2.21 —
  era temporária, só pra destravar a migração de Morango sem acesso
  remoto às máquinas. Essa versão volta a ser "limpa" (sem redirect
  embutido pra nenhum servidor específico).

## 0.2.21 (2026-08-24) — **build especial, só pra migração Contabo → stage-gang**

- Migração automática de servidor embutida no binário: depois de um
  update de launcher aplicado com sucesso (self-check OK), reescreve
  `launcher_settings.json` (`cs_url`/`game_registry_url` → stage-gang)
  sozinho e reinicia o serviço — usado porque não havia acesso remoto
  (SSH) às máquinas físicas de Morango pra editar isso na mão.
  Hardcoded, unidirecional (só Contabo → stage-gang), removido na 0.2.22.

## 0.2.20 (2026-08-24)

- Fix: comando `update_launcher` do heartbeat não pulava mais só por
  bater o número da versão — `apply_update` já decide por hash
  (`already_ready`), então republicar a mesma versão com conteúdo
  diferente agora é aplicado corretamente. Também corrige o bug de
  `pending_launcher_version` ficar preso pra sempre quando a versão já
  batia (o launcher nunca reportava de volta pra CS nesse caso,
  bloqueando qualquer outro comando — game update, restart, reboot —
  atrás dele).

## 0.2.19 (2026-08-24)

- CA interna própria embutida no binário (`tls.rs`) — permite TLS de
  verdade contra servidores sem domínio público (só IP fixo, ex:
  stage-gang), sem precisar instalar certificado no trust store do SO
  de cada máquina. Validação de cadeia continua real, só reconhece mais
  uma raiz confiável além das públicas do sistema.
- Aplicado em todos os clientes HTTP que falam com CS/registry
  (`api::build_client`, `api::download_bytes_with_progress`,
  `update::download_and_extract`, `update::report_update`) — exceto o
  check de conectividade genérico (`https://1.1.1.1`), que continua só
  com as CAs públicas.

## 0.2.18 (2026-08-24)

- `rgs_url`/`rgs_port` (que a CS já devolvia no `/launcher` mas eram
  campos mortos, nunca lidos) agora são combinados e passados pro jogo
  via `--rgs-url` no spawn — mesmo padrão já usado pro `--token`. O jogo
  (ver `RuntimeConfig.hx::applyRgsUrl`, cliente) passa a saber
  dinamicamente qual RGS usar, sem depender só do `--env=prod|stage|
  local` fixo compilado no binário do jogo. Permite trocar de backend
  sem rebuild do jogo.
