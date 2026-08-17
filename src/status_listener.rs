use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use log::{info, warn};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;

pub const DEFAULT_PORT: u16 = 8766;
const MAX_LINE_LENGTH: usize = 256;

type SharedEventBuffer = Arc<Mutex<Vec<(String, DateTime<Utc>)>>>;
type SharedLatestState = Arc<Mutex<Option<(String, DateTime<Utc>)>>>;

/// Dois canais compartilhados sobre o mesmo fluxo de códigos que o jogo
/// manda: `buffer` acumula histórico completo (drenado a cada ~2min pro
/// `/machine/game_events`, ver `main.rs`), `latest` guarda só o código mais
/// recente (sobrescrito, lido pelo heartbeat de 30s sem drenar nada).
/// Clonável (cada campo já é um `Arc` por dentro) — passa por valor entre
/// funções sem precisar duplicar parâmetro em toda a cadeia de chamadas.
#[derive(Clone)]
pub struct GameStatusChannels {
    buffer: SharedEventBuffer,
    latest: SharedLatestState,
}

impl GameStatusChannels {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
            latest: Arc::new(Mutex::new(None)),
        }
    }

    /// Drena o buffer (esvazia) pro canal de histórico mandar de uma vez —
    /// mesma leva não aparece duas vezes se o envio falhar depois de já ter
    /// drenado (ponytail: perde no máximo 1 leva se a rede cair nesse
    /// instante exato).
    pub fn drain(&self) -> Vec<(String, DateTime<Utc>)> {
        std::mem::take(&mut *self.buffer.lock().unwrap())
    }

    /// Lê o último código sem drenar nada (não destrutivo) — pro heartbeat
    /// incluir no corpo a cada ciclo, independente do canal de histórico.
    pub fn peek_latest(&self) -> Option<(String, DateTime<Utc>)> {
        self.latest.lock().unwrap().clone()
    }
}

/// Aceita conexões do jogo (`LauncherStatusBridge.hx`), lê 1 código por linha
/// (5 dígitos: ligado/crédito/atividade/detalhe/ação) e empilha no buffer de
/// histórico + sobrescreve o último código. Só o jogo escreve — launcher
/// nunca responde nada na conexão. Loop de aceitar roda pra sempre; jogo
/// pode reconectar quantas vezes quiser (crash/restart), cada conexão nova
/// só segue lendo linha por linha até fechar.
pub async fn run(channels: GameStatusChannels, port: u16) {
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
        let channels = channels.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let code = line.trim();
                        if code.is_empty() || code.len() > MAX_LINE_LENGTH {
                            continue;
                        }
                        let entry = (code.to_string(), Utc::now());
                        channels.buffer.lock().unwrap().push(entry.clone());
                        *channels.latest.lock().unwrap() = Some(entry);
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    #[tokio::test]
    async fn buffer_keeps_every_transition_latest_keeps_only_last() {
        let channels = GameStatusChannels::new();
        let port = 18766; // porta fixa de teste, não a DEFAULT_PORT de produção
        tokio::spawn(run(channels.clone(), port));
        // dá tempo do listener abrir a porta antes do connect
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        for code in ["11000", "11100", "11101", "11100", "11148"] {
            stream.write_all(format!("{}\n", code).as_bytes()).await.unwrap();
        }
        stream.flush().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // peek_latest não drena — código mais recente, buffer intacto
        let (latest_code, _) = channels.peek_latest().expect("devia ter um valor");
        assert_eq!(latest_code, "11148");

        let drained = channels.drain();
        let codes: Vec<&str> = drained.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(codes, vec!["11000", "11100", "11101", "11100", "11148"]);

        // drain esvazia — segunda chamada vem vazia
        assert!(channels.drain().is_empty());
        // peek_latest continua não-destrutivo mesmo depois do drain
        assert_eq!(channels.peek_latest().unwrap().0, "11148");
    }
}
