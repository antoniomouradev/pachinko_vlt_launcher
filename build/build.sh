#!/bin/bash
set -e

echo "Building Pachinko VLT Launcher for Linux..."

# Build release
cargo build --release

# Cria diretório de distribuição
mkdir -p dist/linux
cp target/release/pachinko_vlt_launcher dist/linux/
cp service/pachinko-launcher.service dist/linux/
cp service/install_service.sh dist/linux/
chmod +x dist/linux/install_service.sh
chmod +x dist/linux/pachinko_vlt_launcher

echo "✅ Build completo! Binário em: dist/linux/pachinko_vlt_launcher"
echo "   Para instalar como serviço, execute: dist/linux/install_service.sh"
