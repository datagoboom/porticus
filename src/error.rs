use thiserror::Error;

/// Top-level error type for the bridge. Each variant wraps the underlying error
/// from the relevant layer (serial, WebSocket, or raw I/O) via `#[from]`, so
/// the `?` operator converts automatically.
#[derive(Debug, Error)]
pub enum PorticusError {
    #[error("Serial port error: {0}")]
    SerialPort(#[from] tokio_serial::Error),
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
