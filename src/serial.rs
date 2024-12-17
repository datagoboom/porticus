use tokio::sync::Mutex;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_serial::SerialPortBuilderExt;
use crate::config::PorticusConfig;
use crate::error::PorticusError;

pub async fn create_serial_port(
    config: &PorticusConfig,
) -> Result<Arc<Mutex<dyn tokio_serial::SerialPort>>, PorticusError> {
    let serial_port = tokio_serial::new(&config.serial_port, config.baud_rate)
        .open_native_async()?;
    Ok(Arc::new(Mutex::new(serial_port)))
}

pub async fn handle_serial_reading(
    serial_reader: Arc<Mutex<dyn tokio_serial::SerialPort>>,
    tx: Arc<broadcast::Sender<Vec<u8>>>,
    buffer_size: usize,
    quiet: bool,
) {
    let mut buffer = vec![0u8; buffer_size];
    
    loop {
        match serial_reader.lock().await.read(&mut buffer) {
            Ok(n) if n > 0 => {
                for byte in &buffer[..n] {
                    let _ = tx.send(vec![*byte]);
                }
            }
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                continue;
            }
            Err(e) => {
                if !quiet {
                    eprintln!("Error reading from serial port: {}", e);
                }
                break;
            }
        }
    }
}
