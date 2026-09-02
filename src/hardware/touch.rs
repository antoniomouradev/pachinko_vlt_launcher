use std::process::Command;

/// USB ID do touchscreen físico da tela de baixo (E-Box, "CSCTEK USB Audio
/// and HID" combinado com touch) nas VLTs de tela dupla. Achado 28/08
/// (ver memória `project_touch_dp2_calibration_28_08`): sem essa
/// `InputClass`, o range analógico do sensor (0-4095) é espalhado pelo
/// desktop virtual INTEIRO (as duas telas juntas) em vez de restringir só
/// à metade de baixo (DP-2) onde o painel fica fisicamente — resultado:
/// metade do curso do dedo cai na tela errada (a de cima).
const EBOX_TOUCH_USB_ID: &str = "04e7:0007";

const CONF_PATH: &str = "/etc/X11/xorg.conf.d/51-ebox-touch-dp2.conf";

const CONF_CONTENT: &str = r#"Section "InputClass"
    Identifier "E-Box Touchscreen -> DP-2 (bottom half)"
    MatchUSBID "04e7:0007"
    Driver "libinput"
    Option "TransformationMatrix" "1 0 0 0 0.5 0.5 0 0 1"
EndSection
"#;

/// Só parsing puro (sem I/O), testável sem `lsusb` real.
fn lsusb_has_id(output: &str, usb_id: &str) -> bool {
    output.lines().any(|line| line.contains(usb_id))
}

fn detect_ebox_touch() -> bool {
    match Command::new("lsusb").output() {
        Ok(o) if o.status.success() => {
            lsusb_has_id(&String::from_utf8_lossy(&o.stdout), EBOX_TOUCH_USB_ID)
        }
        Ok(o) => {
            log::warn!(
                "lsusb saiu com status {} detectando touch E-Box: {}",
                o.status,
                String::from_utf8_lossy(&o.stderr)
            );
            false
        }
        Err(e) => {
            log::warn!("Falha ao rodar lsusb pra detectar touch E-Box: {}", e);
            false
        }
    }
}

/// Garante a calibração do touch da tela de baixo pra máquinas com o
/// painel E-Box (`04e7:0007`) — mesmo padrão do auto-fix de volume
/// (`hardware::audio::set_volume`): best-effort, loga e segue se o
/// dispositivo não existir nessa máquina, nunca trava o boot.
///
/// Idempotente: só escreve se o conteúdo atual do arquivo divergir do
/// esperado (evita reescrever à toa em toda subida).
///
/// **Efeito só no PRÓXIMO boot do X** — `xorg.conf.d` não tem hot-reload,
/// e o launcher já roda como cliente X (depois do X já ter subido), então
/// não dá pra aplicar na sessão atual sem reiniciar o X — o que
/// derrubaria o jogador na hora. Por isso essa função nunca reinicia o X
/// sozinha; se o arquivo mudou agora, só passa a valer depois de um
/// reboot normal da máquina.
pub fn ensure_touch_calibration() {
    if !detect_ebox_touch() {
        log::debug!("Touch E-Box ({}) não detectado, pulando calibração.", EBOX_TOUCH_USB_ID);
        return;
    }

    let already_correct = std::fs::read_to_string(CONF_PATH)
        .map(|existing| existing == CONF_CONTENT)
        .unwrap_or(false);

    if already_correct {
        log::debug!("Calibração do touch E-Box já aplicada ({}).", CONF_PATH);
        return;
    }

    if let Some(parent) = std::path::Path::new(CONF_PATH).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!("Falha ao garantir diretório {}: {}", parent.display(), e);
            return;
        }
    }

    match std::fs::write(CONF_PATH, CONF_CONTENT) {
        Ok(()) => log::info!(
            "Calibração do touch E-Box escrita em {} (efeito no próximo boot do X).",
            CONF_PATH
        ),
        Err(e) => log::warn!("Falha ao escrever calibração do touch E-Box em {}: {}", CONF_PATH, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LSUSB: &str = "\
Bus 001 Device 003: ID 04e7:0007 E-Box Touchscreen
Bus 001 Device 002: ID 8087:0024 Intel Corp.
Bus 001 Device 001: ID 1d6b:0002 Linux Foundation 2.0 root hub";

    #[test]
    fn detects_ebox_id_when_present() {
        assert!(lsusb_has_id(SAMPLE_LSUSB, EBOX_TOUCH_USB_ID));
    }

    #[test]
    fn does_not_detect_when_absent() {
        let output = "\
Bus 001 Device 002: ID 8087:0024 Intel Corp.
Bus 001 Device 001: ID 1d6b:0002 Linux Foundation 2.0 root hub";
        assert!(!lsusb_has_id(output, EBOX_TOUCH_USB_ID));
    }

    #[test]
    fn empty_output_is_not_detected() {
        assert!(!lsusb_has_id("", EBOX_TOUCH_USB_ID));
    }
}
