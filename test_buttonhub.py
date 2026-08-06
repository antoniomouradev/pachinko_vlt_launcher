#!/usr/bin/env python3
"""Teste manual de botões via buttonhub, sem precisar do launcher.

Conecta em 127.0.0.1:8765 (mesmo protocolo que o launcher usa,
ver src/hardware/buttonhub.rs) e imprime cada evento cru assim que chega.

Uso: python3 test_buttonhub.py [porta]
Ctrl+C para sair.
"""
import socket
import sys
from datetime import datetime

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8765

# Mapa de teclas conhecido do TUI (src/tui.rs map_button())
KEY_MAP = {
    "ki": "ENTER",
    "ke": "ESC",
    "ka": "UP",
    "kg": "DOWN",
}


def describe(line: str) -> str:
    if line in KEY_MAP:
        return f"tecla {line} -> {KEY_MAP[line]}"
    if line.startswith("$"):
        return f"noteiro: {line[1:]}"
    if line == "!":
        return "heartbeat (offline?)"
    return "evento desconhecido"


def main() -> None:
    print(f"Conectando em 127.0.0.1:{PORT} ...")
    with socket.create_connection(("127.0.0.1", PORT)) as sock:
        print("Conectado. Aperta os botões (Ctrl+C sai).\n")
        buf = b""
        while True:
            chunk = sock.recv(1024)
            if not chunk:
                print("Conexão fechada pelo buttonhub.")
                break
            buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                text = line.decode(errors="replace").strip()
                if not text:
                    continue
                ts = datetime.now().strftime("%H:%M:%S.%f")[:-3]
                print(f"[{ts}] {text!r:10} {describe(text)}")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\nSaindo.")
    except ConnectionRefusedError:
        print(f"Não conectou na porta {PORT} — buttonhub tá rodando?")
        sys.exit(1)
