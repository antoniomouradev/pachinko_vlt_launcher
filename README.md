# Pachinko VLT Launcher

Launcher para registro e autenticação de máquinas VLT no servidor RGS.

## Visão Geral

O launcher VLT é responsável por:
- Coletar informações de hardware do sistema (MAC, UUID, disk serials, CPU, hostname)
- Registrar a máquina no servidor RGS via HTTP
- Salvar `machine_id` e `registration_pin` em arquivo de configuração local
- Pode ser instalado como serviço systemd para executar no boot

## Pré-requisitos

- Rust toolchain (1.70 ou superior)
  - Instale via: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- Sistema Linux com systemd (para instalação como serviço)

## Build

### Build Local

```bash
cargo build --release
```

O binário será gerado em `target/release/pachinko_vlt_launcher`.

### Escolhendo o ambiente (vlt → Contabo, street → stage-gang, ...)

URLs padrão (`CS_URL`/`GAME_REGISTRY_URL`, usadas quando essas env vars não
estão setadas em runtime — inclusive no serviço systemd instalado pelo
launcher) são escolhidas em tempo de **compilação** via `LAUNCHER_ENV`. Bate
1:1 com a variante física da máquina (`vlt`/`street`): sem a env var (ou com
`LAUNCHER_ENV=vlt`) cai no Contabo, `LAUNCHER_ENV=street` cai no stage-gang:

```bash
cargo build --release                    # vlt → Contabo (padrão)
LAUNCHER_ENV=street cargo build --release # street → stage-gang
```

Ambiente novo (fora `vlt`/`street`) = adicionar uma linha em cada função de
`src/env_config.rs`.

### Build com Script

```bash
./build/build.sh
```

O script cria um diretório `dist/linux/` com:
- `pachinko_vlt_launcher` - Binário compilado
- `pachinko-launcher.service` - Arquivo systemd
- `install_service.sh` - Script de instalação

## Uso

### Registro Normal

Executa o launcher normalmente. Se não houver configuração, ele coletará hardware e registrará a máquina:

```bash
./pachinko_vlt_launcher
```

### Forçar Novo Registro

Força um novo registro mesmo se já existir configuração:

```bash
./pachinko_vlt_launcher --register
```

### Verificar Status

Verifica o status do registro atual:

```bash
./pachinko_vlt_launcher --check
```

### Iniciar Jogo

Inicia o jogo com as credenciais (access_token e signing_secret) passadas via variáveis de ambiente:

```bash
./pachinko_vlt_launcher --launch
```

**Nota**: Requer que a máquina esteja aprovada (status ACTIVE) e tenha tokens configurados.

### Instalar como Serviço

Instala o launcher como serviço systemd para executar no boot:

```bash
sudo ./pachinko_vlt_launcher --install-service
```

**Nota**: Requer privilégios de root.

### Remover Serviço

Remove o serviço do systemd:

```bash
sudo ./pachinko_vlt_launcher --uninstall-service
```

## Configuração

### Arquivo de Configuração

O launcher salva a configuração em:
- Linux: `~/.config/pachinko/launcher_config.json`

Estrutura do arquivo:

```json
{
  "machine_id": 123,
  "registration_pin": "123456",
  "registered_at": "2026-01-15T10:30:00Z",
  "rgs_url": "http://localhost:43310",
  "hardware_fingerprint": "abc123...",
  "access_token": "encrypted_jwt_token...",
  "signing_secret": "encrypted_hmac_secret..."
}
```

**Nota**: Os campos `access_token` e `signing_secret` são salvos criptografados e só aparecem após aprovação da máquina.

### Variáveis de Ambiente

- `RGS_URL`: URL do servidor RGS (padrão: `http://localhost:43310`)
- `RUST_LOG`: Nível de logging (padrão: `info`, opções: `error`, `warn`, `info`, `debug`, `trace`)

Exemplo:

```bash
export RGS_URL="http://localhost:43310"
export RUST_LOG="debug"
./pachinko_vlt_launcher
```

## Fluxo de Registro

1. **Primeira Execução**:
   - Launcher coleta informações de hardware
   - Faz POST para `{RGS_URL}/api/vlt/register_machine`
   - Recebe `machine_id` e `registration_pin`
   - Salva em `launcher_config.json`
   - Se `requires_approval=true`, mostra PIN e aguarda aprovação

2. **Execuções Subsequentes**:
   - Launcher lê `machine_id` de `launcher_config.json`
   - Valida que máquina está registrada
   - Pronto para uso

3. **Re-registro**:
   - Use `--register` para forçar novo registro
   - Útil após mudanças no hardware

## Instalação como Serviço

### Método 1: Usando o Launcher

```bash
sudo ./pachinko_vlt_launcher --install-service
```

### Método 2: Usando Script de Instalação

Se você usou `build.sh`, pode instalar usando o script:

```bash
cd dist/linux
sudo ./install_service.sh
```

### Gerenciar Serviço

```bash
# Ver status
sudo systemctl status pachinko-launcher

# Ver logs
sudo journalctl -u pachinko-launcher -f

# Parar serviço
sudo systemctl stop pachinko-launcher

# Iniciar serviço
sudo systemctl start pachinko-launcher

# Reiniciar serviço
sudo systemctl restart pachinko-launcher

# Desabilitar serviço (não inicia no boot)
sudo systemctl disable pachinko-launcher
```

## Integração com Jogo

O launcher passa credenciais para o jogo via **argumentos de linha de comando**:

**Argumentos passados ao jogo:**
- `--vlt_machine_id=5`: ID da máquina
- `--vlt_rgs_url=http://localhost:43310`: URL do servidor RGS
- `--vlt_access_token=eyJhbGci...`: JWT token (quando máquina está ACTIVE)
- `--vlt_signing_secret=abc123...`: HMAC secret (quando máquina está ACTIVE)

**Exemplo de uso no jogo (Haxe):**
```haxe
// Ler argumentos de linha de comando
var args = Sys.args();
var machineId = getArgValue(args, "--vlt_machine_id");
var rgsUrl = getArgValue(args, "--vlt_rgs_url");
var accessToken = getArgValue(args, "--vlt_access_token");
var signingSecret = getArgValue(args, "--vlt_signing_secret");

function getArgValue(args:Array<String>, flag:String):String {
    for (arg in args) {
        if (arg.startsWith(flag + "=")) {
            return arg.substring(flag.length + 1);
        }
    }
    return null;
}
```

O jogo usa `access_token` no comando WebSocket `open`:

```json
{
  "type": "open",
  "payload": {
    "channel": "vlt",
    "game": "pachinko3",
    "machine_id": 123,
    "access_token": "eyJhbGciOiJIUzI1Ni..."
  }
}
```

**Para comandos críticos** (`play`, `end`, `descredit`), o jogo deve assinar o payload com HMAC:
1. Construir mensagem canônica: `timestamp:{"campo1":"valor1","campo2":"valor2"}` (ordenado)
2. Calcular HMAC-SHA256 usando `VLT_SIGNING_SECRET`
3. Incluir `hmac_signature` e `timestamp` no payload

```json
{
  "type": "play",
  "payload": {
    "bet": 4,
    "active_cards": [0, 1, 2, 3],
    "hmac_signature": "abc123...",
    "timestamp": "2026-01-15T10:30:00Z"
  }
}
```

## Troubleshooting

### Erro: "Connection refused" ou "Falha ao conectar ao servidor RGS"

Este erro significa que o launcher não conseguiu conectar ao servidor RGS. Verifique:

1. **Servidor RGS está rodando?**
   ```bash
   # Verifique se o servidor está respondendo
   curl http://localhost:43310/health
   # ou
   curl http://seu-servidor:43310/health
   ```

2. **URL do RGS está correta?**
   ```bash
   # Configure a URL correta antes de executar
   export RGS_URL="http://seu-servidor-rgs:43310"
   ./target/release/pachinko_vlt_launcher
   ```
   
   **Nota:** O padrão é `http://localhost:43310`. Se seu servidor RGS está em outra porta ou host, defina `RGS_URL`.

3. **Porta está acessível?**
   - Verifique se não há firewall bloqueando
   - Se o RGS está em Docker, verifique se a porta está mapeada corretamente
   - Teste a conectividade: `telnet seu-servidor 43310` ou `nc -zv seu-servidor 43310`

4. **Verificar logs detalhados:**
   ```bash
   RUST_LOG=debug ./target/release/pachinko_vlt_launcher
   ```

### Erro: "Falha ao coletar informações de hardware"

- **Linux**: Verifique permissões de leitura em `/sys/class/net`, `/etc/machine-id`, `/proc/cpuinfo`
- **Linux**: Certifique-se de que `lsblk` está instalado
- **macOS**: Verifique se `ifconfig`, `ioreg`, `diskutil` e `sysctl` estão disponíveis

### Erro: "Falha ao registrar máquina" (HTTP 4xx/5xx)

- Verifique se o endpoint `/api/vlt/register_machine` está disponível no servidor RGS
- Verifique logs do servidor RGS para ver o erro específico
- Verifique se o payload está sendo enviado corretamente (use `RUST_LOG=debug`)

### Erro: "Instalação de serviço requer privilégios de root"

- Execute com `sudo`:
  ```bash
  sudo ./pachinko_vlt_launcher --install-service
  ```

### Serviço não inicia no boot

- Verifique se está habilitado:
  ```bash
  sudo systemctl is-enabled pachinko-launcher
  ```
- Se não estiver, habilite:
  ```bash
  sudo systemctl enable pachinko-launcher
  ```

### Ver logs do serviço

```bash
sudo journalctl -u pachinko-launcher -f
```

## Testes e Execução Local

### Compilar o Projeto

```bash
cd pachinko_vlt_launcher
cargo build --release
```

O binário será gerado em `target/release/pachinko_vlt_launcher`.

### Executar para Testes (sem instalar)

Após compilar, você pode executar diretamente:

```bash
# Executar normalmente (registra se não tiver config)
./target/release/pachinko_vlt_launcher

# Ou copiar para um local mais conveniente
cp target/release/pachinko_vlt_launcher .
./pachinko_vlt_launcher
```

### Configurar URL do RGS para Testes

```bash
# Definir URL do servidor RGS
export RGS_URL="http://localhost:43310"

# Executar com logs detalhados
export RUST_LOG=debug
./target/release/pachinko_vlt_launcher
```

### Comandos de Teste

```bash
# Verificar status do registro
./target/release/pachinko_vlt_launcher --check

# Forçar novo registro
./target/release/pachinko_vlt_launcher --register

# Executar normalmente (registra se necessário)
./target/release/pachinko_vlt_launcher
```

### Localização do Arquivo de Configuração

O arquivo de configuração será criado em:
- Linux: `~/.config/pachinko/launcher_config.json`

Você pode verificar/editá-lo manualmente se necessário:

```bash
cat ~/.config/pachinko/launcher_config.json
```

### Testar sem Servidor RGS

Se você quiser testar apenas a coleta de hardware (sem registrar no servidor), pode:

1. Executar normalmente - ele tentará conectar ao RGS e falhará
2. Verificar os logs para ver as informações coletadas
3. O arquivo de config não será criado se o registro falhar

## Desenvolvimento

### Estrutura do Projeto

```
pachinko_vlt_launcher/
├── Cargo.toml              # Configuração do projeto
├── src/
│   ├── main.rs            # Entry point
│   ├── error.rs           # Tipos de erro
│   ├── hardware/          # Coleta de hardware
│   │   ├── mod.rs
│   │   └── linux.rs
│   ├── api/               # Cliente HTTP
│   │   └── mod.rs
│   ├── config/            # Gerenciamento de config
│   │   └── mod.rs
│   └── service/           # Instalação como serviço
│       ├── mod.rs
│       └── linux.rs
├── build/
│   └── build.sh          # Script de build
└── service/
    ├── pachinko-launcher.service
    └── install_service.sh
```

### Executar Testes

```bash
cargo test
```

### Executar com Logs Detalhados

```bash
RUST_LOG=debug ./target/release/pachinko_vlt_launcher
```

## Licença

PROPRIETARY - Todos os direitos reservados.
