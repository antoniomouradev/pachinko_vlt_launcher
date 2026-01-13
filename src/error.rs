use thiserror::Error;

#[derive(Error, Debug)]
pub enum LauncherError {
    #[error("Erro ao coletar hardware: {0}")]
    HardwareCollectionError(String),

    #[error("Erro de rede: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Erro ao registrar máquina: {0}")]
    RegistrationError(String),

    #[error("Erro de configuração: {0}")]
    ConfigError(String),

    #[error("Erro de serviço: {0}")]
    ServiceError(String),

    #[error("Erro de I/O: {0}")]
    IoError(#[from] std::io::Error),
}
