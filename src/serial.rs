use std::io;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};
use tokio_serial::{SerialPortBuilderExt, SerialStream};
use tracing::{info, warn};

use crate::config::PorticusConfig;

const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

/// Owns the serial port for the lifetime of the process. Reads are fanned out
/// to WebSocket clients through `tx`; writes from clients arrive on `write_rx`.
/// If the device disappears, keeps retrying the open with exponential backoff
/// so clients can stay connected across a replug.
pub async fn run(
    config: PorticusConfig,
    tx: broadcast::Sender<Vec<u8>>,
    mut write_rx: mpsc::Receiver<Vec<u8>>,
) {
    let mut delay = Duration::from_secs(1);
    loop {
        match tokio_serial::new(&config.serial_port, config.baud_rate).open_native_async() {
            Ok(stream) => {
                info!(port = %config.serial_port, baud = config.baud_rate, "serial port opened");
                delay = Duration::from_secs(1);
                match bridge(stream, &tx, &mut write_rx, config.buffer_size).await {
                    Ok(()) => return, // all write senders dropped: shutting down
                    Err(e) => warn!("serial port error: {e}"),
                }
            }
            Err(e) => warn!(port = %config.serial_port, "failed to open serial port: {e}"),
        }
        info!("retrying in {}s", delay.as_secs());
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

/// Pumps one open serial stream: device bytes out through `tx` (one message
/// per read, not per byte), client bytes in from `write_rx`. Returns Ok(())
/// when the write channel closes, Err on device I/O failure.
pub async fn bridge(
    stream: SerialStream,
    tx: &broadcast::Sender<Vec<u8>>,
    write_rx: &mut mpsc::Receiver<Vec<u8>>,
    buffer_size: usize,
) -> io::Result<()> {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut buffer = vec![0u8; buffer_size.max(1)];
    loop {
        tokio::select! {
            read = reader.read(&mut buffer) => {
                let n = read?;
                if n == 0 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "serial port closed"));
                }
                // Ignore send errors: no clients connected is fine.
                let _ = tx.send(buffer[..n].to_vec());
            }
            data = write_rx.recv() => {
                match data {
                    Some(data) => writer.write_all(&data).await?,
                    None => return Ok(()),
                }
            }
        }
    }
}
