use thiserror::Error;

#[derive(Debug, Error)]
pub enum PorticusError {
    #[error("Serial port error: {0}")]
    SerialPort(#[from] tokio_serial::Error),
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
} 