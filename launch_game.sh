#!/bin/bash

# Script de inicialização do Launcher VLT
# Facilita o uso do launcher para gerenciar máquinas VLT

set -e

# Cores para output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Diretório do launcher
LAUNCHER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAUNCHER_BIN="${LAUNCHER_DIR}/target/release/pachinko_vlt_launcher"

# Caminho do jogo (ajuste conforme necessário)
# Para macOS .app bundle, pode ser o .app ou o executável dentro dele
GAME_PATH="${VLT_GAME_PATH:-/Users/amoura/pachinko/Projects/pachinko_game/pachinko_remake/pachinko_game_bingo/Export/macos/bin/PachinkoGameBingo.app}"

# URL do RGS (pode ser sobrescrita por variável de ambiente)
RGS_URL="${RGS_URL:-http://localhost:43310}"

# Função para imprimir mensagens coloridas
print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

print_success() {
    echo -e "${GREEN}✅${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}⚠️${NC} $1"
}

print_error() {
    echo -e "${RED}❌${NC} $1"
}

# Função para verificar se o launcher está compilado
check_launcher() {
    if [ ! -f "$LAUNCHER_BIN" ]; then
        print_error "Launcher não encontrado em: $LAUNCHER_BIN"
        print_info "Compilando launcher..."
        cd "$LAUNCHER_DIR"
        cargo build --release
        if [ $? -ne 0 ]; then
            print_error "Falha ao compilar launcher"
            exit 1
        fi
        print_success "Launcher compilado com sucesso!"
    fi
}

# Função para verificar se o jogo existe
check_game() {
    # Verificar se é um arquivo executável direto
    if [ -f "$GAME_PATH" ] && [ -x "$GAME_PATH" ]; then
        return 0
    fi
    
    # Verificar se é um .app bundle (macOS)
    if [[ "$GAME_PATH" == *.app ]]; then
        # Tentar encontrar o executável dentro do bundle
        APP_BINARY="${GAME_PATH}/Contents/MacOS/$(basename "$GAME_PATH" .app)"
        if [ -f "$APP_BINARY" ] && [ -x "$APP_BINARY" ]; then
            GAME_PATH="$APP_BINARY"
            return 0
        fi
    fi
    
    # Se o caminho aponta para um diretório .app, tentar encontrar o executável
    if [ -d "$GAME_PATH" ] && [[ "$GAME_PATH" == *.app ]]; then
        APP_NAME=$(basename "$GAME_PATH" .app)
        APP_BINARY="${GAME_PATH}/Contents/MacOS/${APP_NAME}"
        if [ -f "$APP_BINARY" ] && [ -x "$APP_BINARY" ]; then
            GAME_PATH="$APP_BINARY"
            return 0
        fi
    fi
    
    # Tentar caminho alternativo comum para .app bundles
    APP_DIR="${GAME_PATH%.app}.app"
    if [ -d "$APP_DIR" ]; then
        APP_NAME=$(basename "$APP_DIR" .app)
        APP_BINARY="${APP_DIR}/Contents/MacOS/${APP_NAME}"
        if [ -f "$APP_BINARY" ] && [ -x "$APP_BINARY" ]; then
            GAME_PATH="$APP_BINARY"
            print_info "Encontrado executável em: $GAME_PATH"
            return 0
        fi
    fi
    
    print_warning "Jogo não encontrado em: $GAME_PATH"
    print_info "Configure VLT_GAME_PATH ou ajuste o caminho no script"
    print_info "Para .app bundles no macOS, o executável geralmente está em:"
    print_info "  NomeApp.app/Contents/MacOS/NomeApp"
    return 1
}

# Função para mostrar menu
show_menu() {
    echo ""
    echo "=========================================="
    echo "  Pachinko VLT Launcher - Menu"
    echo "=========================================="
    echo ""
    echo "1. Verificar status do registro"
    echo "2. Registrar/Atualizar máquina"
    echo "3. Iniciar jogo"
    echo "4. Verificar e iniciar (automático)"
    echo "5. Sair"
    echo ""
    echo -n "Escolha uma opção [1-5]: "
}

# Função para verificar status
check_status() {
    print_info "Verificando status do registro..."
    cd "$LAUNCHER_DIR"
    export RGS_URL="$RGS_URL"
    "$LAUNCHER_BIN" check
}

# Função para registrar
register() {
    print_info "Registrando/Atualizando máquina no RGS..."
    cd "$LAUNCHER_DIR"
    export RGS_URL="$RGS_URL"
    "$LAUNCHER_BIN" register
}

# Função para iniciar o jogo
launch() {
    if ! check_game; then
        print_error "Não é possível iniciar o jogo. Caminho inválido."
        return 1
    fi
    
    print_info "Iniciando jogo..."
    print_info "Caminho do jogo: $GAME_PATH"
    print_info "RGS URL: $RGS_URL"
    
    cd "$LAUNCHER_DIR"
    export RGS_URL="$RGS_URL"
    export VLT_GAME_PATH="$GAME_PATH"
    
    "$LAUNCHER_BIN" launch
    
    if [ $? -eq 0 ]; then
        print_success "Jogo iniciado com sucesso!"
    else
        print_error "Falha ao iniciar jogo"
        print_info "Verifique se a máquina está aprovada e tem tokens configurados"
    fi
}

# Função para verificar e iniciar automaticamente
check_and_launch() {
    print_info "Verificando status antes de iniciar..."
    check_status
    
    echo ""
    print_info "Iniciando jogo..."
    launch
}

# Função principal
main() {
    # Verificar se o launcher está compilado
    check_launcher
    
    # Se argumentos foram passados, executar diretamente
    case "${1:-}" in
        check|status)
            check_status
            ;;
        register|reg)
            register
            ;;
        launch|start)
            launch
            ;;
        auto)
            check_and_launch
            ;;
        "")
            # Modo interativo
            while true; do
                show_menu
                read -r choice
                case $choice in
                    1)
                        check_status
                        ;;
                    2)
                        register
                        ;;
                    3)
                        launch
                        ;;
                    4)
                        check_and_launch
                        ;;
                    5)
                        print_info "Saindo..."
                        exit 0
                        ;;
                    *)
                        print_error "Opção inválida. Tente novamente."
                        ;;
                esac
                echo ""
                read -p "Pressione Enter para continuar..."
            done
            ;;
        *)
            echo "Uso: $0 [check|register|launch|auto]"
            echo ""
            echo "Comandos:"
            echo "  check, status    - Verifica status do registro"
            echo "  register, reg     - Registra/Atualiza máquina"
            echo "  launch, start     - Inicia o jogo"
            echo "  auto              - Verifica status e inicia automaticamente"
            echo ""
            echo "Sem argumentos: abre menu interativo"
            echo ""
            echo "Variáveis de ambiente:"
            echo "  VLT_GAME_PATH     - Caminho do executável do jogo"
            echo "  RGS_URL           - URL do servidor RGS (padrão: http://localhost:43310)"
            exit 1
            ;;
    esac
}

# Executar função principal
main "$@"
