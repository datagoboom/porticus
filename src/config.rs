//! Runtime configuration for the bridge. Built in `main` from defaults overlaid
//! with CLI flags, then handed to [`crate::serial`] and [`crate::websocket`].

use crate::framing::Framing;

/// All tunables for a running bridge. Every field has a sensible default
/// (see the [`Default`] impl); the CLI overrides individual fields.
#[derive(Debug, Clone)]
pub struct PorticusConfig {
    /// Serial device path (e.g. `/dev/ttyACM0`, `COM1`).
    pub serial_port: String,
    /// Serial baud rate.
    pub baud_rate: u32,
    /// TCP port the WebSocket server listens on.
    pub websocket_port: u16,
    /// Host/interface the WebSocket server binds to.
    pub websocket_host: String,
    /// Serial read buffer size in bytes; also the max size of one fanned-out
    /// message (one broadcast message per read, capped at this many bytes).
    pub buffer_size: usize,
    /// Broadcast channel capacity: serial reads buffered per client before a
    /// slow client starts dropping messages (logged as "client too slow").
    pub broadcast_capacity: usize,
    /// Write channel capacity: client-to-device messages buffered before send
    /// back-pressures. Messages queued here survive a device reconnect and are
    /// flushed once the port reopens.
    pub write_capacity: usize,
    /// How the serial byte stream is split into messages (and outgoing messages
    /// wrapped). Defaults to [`Framing::Raw`] (passthrough).
    pub framing: Framing,
    /// Port for the browser console's HTTP server (served on `websocket_host`).
    /// `0` disables the console entirely.
    pub http_port: u16,
}

impl Default for PorticusConfig {
    fn default() -> Self {
        Self {
            serial_port: if cfg!(windows) {
                "COM1".to_string()
            } else {
                "/dev/ttyACM0".to_string()
            },
            baud_rate: 9600,
            websocket_port: 8080,
            websocket_host: "127.0.0.1".to_string(),
            buffer_size: 1024,
            broadcast_capacity: 16,
            write_capacity: 32,
            framing: Framing::Raw,
            http_port: 8081,
        }
    }
}
