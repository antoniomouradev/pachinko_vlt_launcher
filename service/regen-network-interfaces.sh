#!/bin/sh
# Detecta a(s) interface(s) ethernet física(s) reais (ignora `lo` e
# interfaces sem dispositivo de hardware por trás, tipo docker0/veth) e
# escreve config de DHCP pra elas em /etc/network/interfaces.d/ — nome de
# interface muda por motherboard/slot PCI (visto: `eno1` na máquina de
# referência, `enp1s0`/`enp2s0` numa clonada em hardware diferente), então
# não dá pra hardcodar no `/etc/network/interfaces` da imagem golden.
# Idempotente: sobrescreve sempre, sem custo repetir todo boot.
set -eu

OUT=/etc/network/interfaces.d/99-auto-ethernet

: > "$OUT"

for iface_path in /sys/class/net/*/; do
    iface=$(basename "$iface_path")
    [ "$iface" = "lo" ] && continue
    [ -e "${iface_path}device" ] || continue
    # ponytail: pega qualquer interface com hardware real por trás — não
    # distingue ethernet de wifi aqui de propósito, `allow-hotplug` +
    # `dhcp` é seguro nos dois casos (wifi sem credencial configurada
    # simplesmente nunca sobe, sem efeito colateral).
    {
        echo "allow-hotplug $iface"
        echo "iface $iface inet dhcp"
    } >> "$OUT"
done

exit 0
