use std::io::{BufRead, BufReader};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread;

pub const DEFAULT_PORT: u16 = 8765;

/// Conecta no `buttonhub` (ver `interfaces/INTERFACE.md`) e devolve cada
/// evento cru (linha ASCII: `kX`/`KX` tecla, `$NNN` noteiro, `!` heartbeat
/// offline) assim que chega. Sem seleção de driver — usa o que já estiver
/// ativo por padrão (config do script de xinit). Não decodifica nada: é só
/// teste visual de "chegou evento ou não".
pub struct Connection {
    pub events: Receiver<String>,
    stream: TcpStream,
}

impl Connection {
    pub fn close(&self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

pub fn connect(port: u16) -> std::io::Result<Connection> {
    let stream = TcpStream::connect(("127.0.0.1", port))?;
    let reader_stream = stream.try_clone()?;
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        for line in BufReader::new(reader_stream).lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    Ok(Connection { events: rx, stream })
}
