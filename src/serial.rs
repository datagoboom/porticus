use std::io;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};
use tokio_serial::{SerialPortBuilderExt, SerialStream};
use tracing::{info, warn};

use crate::capture::Dir;
use crate::config::PorticusConfig;
use crate::framing::{Framer, Framing};

const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

/// Optional recorder tap: logical messages in both directions are forwarded here
/// (best-effort) for `--record`.
pub type RecordTap = mpsc::Sender<(Dir, Vec<u8>)>;

/// Owns the serial port for the lifetime of the process. Reads are fanned out
/// to WebSocket clients through `tx`; writes from clients arrive on `write_rx`.
/// If the device disappears, keeps retrying the open with exponential backoff
/// so clients can stay connected across a replug. `rec`, when set, receives a
/// copy of every message in both directions for recording.
pub async fn run(
    config: PorticusConfig,
    tx: broadcast::Sender<Vec<u8>>,
    mut write_rx: mpsc::Receiver<Vec<u8>>,
    rec: Option<RecordTap>,
) {
    let mut delay = Duration::from_secs(1);
    loop {
        match tokio_serial::new(&config.serial_port, config.baud_rate).open_native_async() {
            Ok(stream) => {
                info!(port = %config.serial_port, baud = config.baud_rate, "serial port opened");
                delay = Duration::from_secs(1);
                match bridge(
                    stream,
                    &tx,
                    &mut write_rx,
                    config.buffer_size,
                    config.framing,
                    rec.as_ref(),
                )
                .await
                {
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

/// Pumps one open serial stream: device bytes out through `tx`, client bytes in
/// from `write_rx`. The `framing` governs how the byte stream is split into
/// broadcast messages and how outgoing messages are wrapped (see
/// [`crate::framing`]). Returns Ok(()) when the write channel closes, Err on
/// device I/O failure. The framer is created fresh here, so any partial frame is
/// dropped on reconnect rather than straddling two devices. When `rec` is set,
/// each logical message is copied to it for recording (best-effort: a full
/// recorder channel drops the copy rather than stalling the bridge).
pub async fn bridge(
    stream: SerialStream,
    tx: &broadcast::Sender<Vec<u8>>,
    write_rx: &mut mpsc::Receiver<Vec<u8>>,
    buffer_size: usize,
    framing: Framing,
    rec: Option<&RecordTap>,
) -> io::Result<()> {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut framer = Framer::new(framing);
    let mut buffer = vec![0u8; buffer_size.max(1)];
    loop {
        tokio::select! {
            read = reader.read(&mut buffer) => {
                let n = read?;
                if n == 0 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "serial port closed"));
                }
                for frame in framer.decode(&buffer[..n]) {
                    if let Some(r) = rec {
                        let _ = r.try_send((Dir::Rx, frame.clone()));
                    }
                    // Ignore send errors: no clients connected is fine.
                    let _ = tx.send(frame);
                }
            }
            data = write_rx.recv() => {
                match data {
                    Some(data) => {
                        if let Some(r) = rec {
                            let _ = r.try_send((Dir::Tx, data.clone()));
                        }
                        writer.write_all(&framer.encode(&data)).await?
                    }
                    None => return Ok(()),
                }
            }
        }
    }
}
