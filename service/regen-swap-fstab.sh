#!/bin/sh
# Corrige o UUID de swap no /etc/fstab se estiver desatualizado — acontece
# quando a ferramenta de clonagem (Clonezilla/Rescuezilla) recria a
# partição de swap em vez de copiar bit a bit (típico quando o disco de
# destino tem tamanho diferente do original, ou a partição foi
# redimensionada/recriada manualmente). Idempotente: sem swap detectado ou
# UUID já batendo, não faz nada.
set -eu

FSTAB=/etc/fstab

# Acha a(s) partição(ões) de swap reais no disco, pela própria fs type —
# não assume número fixo de partição (varia por hardware/imagem).
REAL_SWAP_UUID=$(blkid -t TYPE=swap -o value -s UUID | head -n1)

[ -n "$REAL_SWAP_UUID" ] || exit 0

FSTAB_SWAP_UUID=$(awk '$3 == "swap" && $1 ~ /^UUID=/ { sub(/^UUID=/, "", $1); print $1; exit }' "$FSTAB")

[ -n "$FSTAB_SWAP_UUID" ] || exit 0

if [ "$FSTAB_SWAP_UUID" != "$REAL_SWAP_UUID" ]; then
    sed -i "s/$FSTAB_SWAP_UUID/$REAL_SWAP_UUID/" "$FSTAB"
    logger -t regen-swap-fstab "UUID de swap desatualizado no fstab (${FSTAB_SWAP_UUID}) — corrigido pra ${REAL_SWAP_UUID}"
fi

exit 0
