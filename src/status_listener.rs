use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use log::{info, warn};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;

pub const DEFAULT_PORT: u16 = 8766;
const MAX_LINE_LENGTH: usize = 256;

pub type SharedEventBuffer = Arc<Mutex<Vec<(String, DateTime<Utc>)>>>;

/// Drena o buffer (esvazia) pro heartbeat mandar de uma vez — mesma leva não
/// aparece duas vezes se o heartbeat falhar depois de já ter drenado
/// (ponytail: perde no máximo 1 leva se a rede cair nesse instante exato).
pub fn drain(buffer: &SharedEventBuffer) -> Vec<(String, DateTime<Utc>)> {
    std::mem::take(&mut *buffer.lock().unwrap())
}

/// Aceita conexões do jogo (`LauncherStatusBridge.hx`), lê 1 código por linha
/// (`e1`..`e5`/`E1`/`E2`) e empilha em `buffer`. Só o jogo escreve — launcher
/// nunca responde nada na conexão. Loop de aceitar roda pra sempre; jogo pode
/// reconectar quantas vezes quiser (crash/restart), cada conexão nova só
/// segue lendo linha por linha até fechar.
pub async fn run(buffer: SharedEventBuffer, port: u16) {
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            warn!("[status-listener] falha ao abrir porta {}: {} — eventos de estado do jogo não serão coletados", port, e);
            return;
        }
    };
    info!("[status-listener] escutando em 127.0.0.1:{}", port);

    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!("[status-listener] erro ao aceitar conexão: {}", e);
                continue;
            }
        };
        let buffer = buffer.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let code = line.trim();
                        if code.is_empty() || code.len() > MAX_LINE_LENGTH {
                            continue;
                        }
                        buffer.lock().unwrap().push((code.to_string(), Utc::now()));
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        });
    }
}
