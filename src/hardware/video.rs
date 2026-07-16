use std::process::Command;

#[derive(Debug, Clone)]
pub struct Display {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
}

/// Saídas conectadas via `xrandr --query`, com posição real na tela virtual
/// (`+X+Y`) — é o que deixa saber qual monitor fica em cima/embaixo/lado a
/// lado (ex: `--below` no script de xinit).
fn list_connected_displays() -> Vec<Display> {
    let output = match Command::new("xrandr").arg("--query").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut displays = Vec::new();

    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 || fields[1] != "connected" {
            continue;
        }
        let name = fields[0].to_string();
        let geometry = fields[2..].iter().find(|f| f.contains('x') && f.contains('+'));

        let Some(geometry) = geometry else { continue };
        let mut parts = geometry.split('+');
        let Some(size) = parts.next() else { continue };
        let (Some(x), Some(y)) = (parts.next(), parts.next()) else { continue };
        let mut size_parts = size.split('x');
        let (Some(w), Some(h)) = (size_parts.next(), size_parts.next()) else { continue };

        if let (Ok(width), Ok(height), Ok(x), Ok(y)) =
            (w.parse(), h.parse(), x.parse(), y.parse())
        {
            displays.push(Display { name, width, height, x, y });
        }
    }

    displays
}

/// Lista de telas pra teste de vídeo — via `xrandr` no ambiente real (X). Sem
/// X (fallback framebuffer ou tamanho de terminal), sintetiza uma única tela
/// "virtual" cobrindo a área toda, já que não dá pra saber layout de verdade.
pub fn get_displays() -> Vec<Display> {
    let displays = list_connected_displays();
    if !displays.is_empty() {
        return displays;
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/sys/class/graphics/fb0/virtual_size") {
            let parts: Vec<&str> = content.trim().split(',').collect();
            if let [w, h] = parts[..] {
                if let (Ok(width), Ok(height)) = (w.parse(), h.parse()) {
                    return vec![Display {
                        name: "fb0".to_string(),
                        width,
                        height,
                        x: 0,
                        y: 0,
                    }];
                }
            }
        }
    }

    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    vec![Display {
        name: "terminal (não é tela física)".to_string(),
        width: width as u32,
        height: height as u32,
        x: 0,
        y: 0,
    }]
}
