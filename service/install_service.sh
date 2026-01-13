#!/bin/bash
set -e

BINARY_PATH="/usr/local/bin/pachinko_vlt_launcher"
SERVICE_FILE="/etc/systemd/system/pachinko-launcher.service"
SERVICE_NAME="pachinko-launcher"

echo "Instalando Pachinko VLT Launcher como serviço..."

# Verifica se está rodando como root
if [ "$EUID" -ne 0 ]; then 
    echo "❌ Erro: Este script requer privilégios de root (sudo)"
    exit 1
fi

# Verifica se o binário existe no diretório atual
if [ ! -f "./pachinko_vlt_launcher" ]; then
    echo "❌ Erro: Binário 'pachinko_vlt_launcher' não encontrado no diretório atual"
    echo "   Execute este script do diretório onde está o binário compilado"
    exit 1
fi

# Copia binário
echo "📦 Copiando binário para $BINARY_PATH..."
cp pachinko_vlt_launcher "$BINARY_PATH"
chmod +x "$BINARY_PATH"

# Copia arquivo de serviço
echo "📦 Copiando arquivo de serviço para $SERVICE_FILE..."
cp pachinko-launcher.service "$SERVICE_FILE"

# Recarrega systemd
echo "🔄 Recarregando systemd..."
systemctl daemon-reload

# Habilita serviço
echo "✅ Habilitando serviço..."
systemctl enable "$SERVICE_NAME"

# Inicia serviço
echo "🚀 Iniciando serviço..."
systemctl start "$SERVICE_NAME"

echo ""
echo "✅ Serviço instalado e iniciado com sucesso!"
echo ""
echo "Comandos úteis:"
echo "   Status: sudo systemctl status $SERVICE_NAME"
echo "   Logs:   sudo journalctl -u $SERVICE_NAME -f"
echo "   Parar:  sudo systemctl stop $SERVICE_NAME"
echo "   Iniciar: sudo systemctl start $SERVICE_NAME"
