#[derive(Debug, Clone)]
pub struct PorticusConfig {
    pub serial_port: String,
    pub baud_rate: u32,
    pub websocket_port: u16,
    pub buffer_size: usize,
    pub broadcast_capacity: usize,
    pub websocket_host: String,
}

impl Default for PorticusConfig {
    fn default() -> Self {
        Self {
            serial_port: if cfg!(windows) { "COM1".to_string() } else { "/dev/ttyACM0".to_string() },
            baud_rate: 9600,
            websocket_port: 8080,
            buffer_size: 1024,
            broadcast_capacity: 16,
            websocket_host: "127.0.0.1".to_string(),
        }
    }
}
