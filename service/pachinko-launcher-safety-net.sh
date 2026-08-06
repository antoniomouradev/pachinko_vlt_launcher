#!/bin/sh
# Rede de segurança fora do processo (ExecStartPre) — roda antes de CADA
# start do serviço. Sem marcador de update pendente, não faz nada (boot
# normal). Com marcador, conta quantos starts seguidos aconteceram com ele
# ainda presente; se passar de um limite, é sinal de que a versão nova nem
# chega a rodar o self-check (crash antes de qualquer log/hardware init) —
# reverte o symlink de fora, sem depender do binário quebrado fazer nada.
set -eu

MARKER=/opt/pachinko-launcher/pending_confirmation
PREVIOUS=/opt/pachinko-launcher/previous_target
SYMLINK=/usr/local/bin/pachinko_vlt_launcher
FAIL_COUNT_FILE=/opt/pachinko-launcher/safety_net_fail_count
MAX_ATTEMPTS=3

if [ ! -f "$MARKER" ]; then
    rm -f "$FAIL_COUNT_FILE"
    exit 0
fi

COUNT=$(cat "$FAIL_COUNT_FILE" 2>/dev/null || echo 0)
COUNT=$((COUNT + 1))
echo "$COUNT" > "$FAIL_COUNT_FILE"

if [ "$COUNT" -ge "$MAX_ATTEMPTS" ] && [ -f "$PREVIOUS" ]; then
    PREV_TARGET=$(cat "$PREVIOUS")
    ln -sfn "$PREV_TARGET" "$SYMLINK"
    rm -f "$MARKER" "$FAIL_COUNT_FILE"
    logger -t pachinko-launcher-safety-net "Update pendente sem confirmação após ${COUNT} starts — revertido pra ${PREV_TARGET}"
fi

exit 0
