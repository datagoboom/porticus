#![cfg(unix)]

use std::ffi::CStr;
use std::fs::File;
use std::os::unix::fs::symlink;
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tokio_serial::SerialStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use porticus::config::PorticusConfig;
use porticus::framing::Framing;
use porticus::{capture, script, serial, websocket};

const WAIT: Duration = Duration::from_secs(5);

/// Spawns the serial bridge (with the given framing) on the slave end of a PTY
/// pair and returns the master end (acting as the "device") plus the bridge's
/// channel endpoints.
fn start_bridge_framed(
    framing: Framing,
) -> (
    SerialStream,
    broadcast::Sender<Vec<u8>>,
    mpsc::Sender<Vec<u8>>,
) {
    let (device, port) = SerialStream::pair().expect("failed to create PTY pair");
    let (tx, _) = broadcast::channel(16);
    let (serial_tx, mut serial_rx) = mpsc::channel(16);
    let bridge_tx = tx.clone();
    tokio::spawn(async move {
        let _ = serial::bridge(port, &bridge_tx, &mut serial_rx, 1024, framing, None).await;
    });
    (device, tx, serial_tx)
}

/// The common case: a raw (passthrough) bridge.
fn start_bridge() -> (
    SerialStream,
    broadcast::Sender<Vec<u8>>,
    mpsc::Sender<Vec<u8>>,
) {
    start_bridge_framed(Framing::Raw)
}

#[tokio::test]
async fn serial_reads_are_broadcast_as_chunks() {
    let (mut device, tx, _serial_tx) = start_bridge();
    let mut rx = tx.subscribe();

    device.write_all(b"hello").await.unwrap();

    let msg = timeout(WAIT, rx.recv()).await.unwrap().unwrap();
    assert_eq!(msg, b"hello");
}

#[tokio::test]
async fn channel_writes_reach_the_serial_port() {
    let (mut device, _tx, serial_tx) = start_bridge();

    serial_tx.send(b"ping".to_vec()).await.unwrap();

    let mut buf = [0u8; 4];
    timeout(WAIT, device.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"ping");
}

#[tokio::test]
async fn line_framing_splits_the_stream_into_lines() {
    let (mut device, tx, serial_tx) = start_bridge_framed(Framing::Line);
    let mut rx = tx.subscribe();

    // Two lines written in a single serial burst must arrive as two separate
    // messages, each without its terminator.
    device.write_all(b"alpha\r\nbeta\n").await.unwrap();
    let first = timeout(WAIT, rx.recv()).await.unwrap().unwrap();
    let second = timeout(WAIT, rx.recv()).await.unwrap().unwrap();
    assert_eq!(first, b"alpha");
    assert_eq!(second, b"beta");

    // Outgoing messages get a newline appended on the way to the device.
    serial_tx.send(b"AT".to_vec()).await.unwrap();
    let mut buf = [0u8; 3];
    timeout(WAIT, device.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"AT\n");
}

async fn start_server() -> (SerialStream, String) {
    let (device, tx, serial_tx) = start_bridge();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = websocket::run(listener, tx, serial_tx).await;
    });
    (device, format!("ws://{addr}"))
}

#[tokio::test]
async fn end_to_end_serial_to_websocket_and_back() {
    let (mut device, url) = start_server().await;
    let (mut client, _) = connect_async(&url).await.unwrap();

    device.write_all(b"from-device").await.unwrap();
    let msg = timeout(WAIT, client.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(msg.into_data().as_ref(), b"from-device");

    client
        .send(Message::binary(b"from-client".as_ref()))
        .await
        .unwrap();
    let mut buf = [0u8; 11];
    timeout(WAIT, device.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"from-client");
}

#[tokio::test]
async fn multiple_clients_receive_the_same_data() {
    let (mut device, url) = start_server().await;
    let (mut client_a, _) = connect_async(&url).await.unwrap();
    let (mut client_b, _) = connect_async(&url).await.unwrap();

    device.write_all(b"fanout").await.unwrap();

    for client in [&mut client_a, &mut client_b] {
        let msg = timeout(WAIT, client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(msg.into_data().as_ref(), b"fanout");
    }
}

/// Opens a fresh pseudo-terminal, returning the master as a `File` and the
/// filesystem path of the slave side (e.g. `/dev/pts/7`). The slave path is
/// what a serial consumer opens; it disappears once the master is closed.
fn open_pty() -> (File, String) {
    unsafe {
        let master: RawFd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(master >= 0, "posix_openpt failed");
        assert_eq!(libc::grantpt(master), 0, "grantpt failed");
        assert_eq!(libc::unlockpt(master), 0, "unlockpt failed");
        let name = libc::ptsname(master);
        assert!(!name.is_null(), "ptsname returned null");
        let path = CStr::from_ptr(name).to_str().unwrap().to_owned();
        (File::from_raw_fd(master), path)
    }
}

/// A unique, stable path under the temp dir for the "device" symlink. porticus
/// opens this path; we repoint it at a new pty to simulate a replug.
fn temp_link() -> std::path::PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("porticus-reconnect-{}-{n}", std::process::id()))
}

/// The headline feature: if the serial device disappears, the bridge keeps
/// retrying the open (with backoff) rather than exiting, and resumes fanning
/// out data once the device returns on the same path — like a USB replug.
#[tokio::test]
async fn serial_reconnects_after_device_disappears() {
    let link = temp_link();

    // Device present: pty #1 behind a stable symlink that porticus will open.
    let (mut master1, pts1) = open_pty();
    symlink(&pts1, &link).unwrap();

    let (tx, mut rx) = broadcast::channel(16);
    // Keep the write sender alive: if it dropped, run() would treat the write
    // channel closing as a clean shutdown and return.
    let (_serial_tx, write_rx) = mpsc::channel(16);
    let cfg = PorticusConfig {
        serial_port: link.to_string_lossy().into_owned(),
        baud_rate: 9600,
        ..PorticusConfig::default()
    };
    let run_handle = tokio::spawn(serial::run(cfg, tx, write_rx, None));

    // First connection: device bytes reach the broadcast channel.
    // (SerialStream also implements std::io::Write, so qualify these File
    // writes to avoid colliding with the tokio AsyncWriteExt calls elsewhere.)
    std::io::Write::write_all(&mut master1, b"phase-one").unwrap();
    let msg = timeout(WAIT, rx.recv())
        .await
        .expect("timed out waiting for first read")
        .unwrap();
    assert_eq!(msg, b"phase-one");

    // Device vanishes: closing the master makes the slave read EOF, and
    // removing the link makes the next open() fail outright.
    drop(master1);
    std::fs::remove_file(&link).unwrap();

    // Stay gone long enough to force at least one failed reopen (backoff is
    // 1s then 2s), proving run() keeps retrying instead of returning.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(
        !run_handle.is_finished(),
        "serial::run exited instead of retrying the open"
    );

    // Device returns on the SAME path: new pty, repointed link.
    let (mut master2, pts2) = open_pty();
    symlink(&pts2, &link).unwrap();

    // A single write (no accumulation to coalesce) must reach the broadcast
    // channel once the bridge reopens — generous window to cover backoff.
    std::io::Write::write_all(&mut master2, b"phase-two").unwrap();
    let msg = timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("timed out waiting for reconnect; bridge did not reopen")
        .unwrap();
    assert_eq!(msg, b"phase-two");

    run_handle.abort();
    let _ = std::fs::remove_file(&link);
}

#[tokio::test]
async fn failed_handshake_does_not_kill_the_server() {
    let (mut device, url) = start_server().await;

    // Not a WebSocket handshake; the server should log it and move on.
    let tcp_addr = url.strip_prefix("ws://").unwrap();
    let mut raw = TcpStream::connect(tcp_addr).await.unwrap();
    raw.write_all(b"GET / HTTP/1.0\r\n\r\n").await.unwrap();
    drop(raw);

    let (mut client, _) = connect_async(&url).await.unwrap();
    device.write_all(b"still-alive").await.unwrap();
    let msg = timeout(WAIT, client.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(msg.into_data().as_ref(), b"still-alive");
}

#[tokio::test]
async fn replay_feeds_recorded_rx_events_to_clients() {
    let path = temp_link();
    // Two rx events (fed to clients) and one tx event (ignored on replay).
    std::fs::write(&path, "0 rx 68656c6c6f\n30 tx 4154\n60 rx 776f726c64\n").unwrap();

    let (tx, mut rx) = broadcast::channel(16);
    let (serial_tx, serial_rx) = mpsc::channel::<Vec<u8>>(16);
    let handle = tokio::spawn(capture::replay(
        tx,
        serial_rx,
        path.to_string_lossy().into_owned(),
    ));

    let first = timeout(WAIT, rx.recv()).await.unwrap().unwrap();
    let second = timeout(WAIT, rx.recv()).await.unwrap().unwrap();
    assert_eq!(first, b"hello");
    assert_eq!(second, b"world");

    drop(serial_tx);
    handle.abort();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn record_captures_both_directions() {
    let path = temp_link();
    let (mut device, port) = SerialStream::pair().unwrap();
    let (tx, _) = broadcast::channel::<Vec<u8>>(16);
    let (serial_tx, mut serial_rx) = mpsc::channel::<Vec<u8>>(16);
    let (rec_tx, rec_rx) = mpsc::channel::<(capture::Dir, Vec<u8>)>(64);

    tokio::spawn(capture::record(rec_rx, path.to_string_lossy().into_owned()));
    let bridge_tx = tx.clone();
    tokio::spawn(async move {
        let _ = serial::bridge(
            port,
            &bridge_tx,
            &mut serial_rx,
            1024,
            Framing::Raw,
            Some(&rec_tx),
        )
        .await;
    });

    device.write_all(b"ping").await.unwrap(); // rx: device -> clients
    serial_tx.send(b"pong".to_vec()).await.unwrap(); // tx: clients -> device
    tokio::time::sleep(Duration::from_millis(300)).await; // let the recorder flush

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("rx 70696e67"),
        "missing rx line in {contents:?}"
    );
    assert!(
        contents.contains("tx 706f6e67"),
        "missing tx line in {contents:?}"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn script_send_expect_passes() {
    let path = temp_link();
    std::fs::write(&path, "# ping/pong\nsend PING\\n\nexpect 2000 PONG\n").unwrap();

    let (mut device, tx, serial_tx) = start_bridge();
    // Device responder: reply "PONG\n" once it sees "PING".
    tokio::spawn(async move {
        let mut buf = [0u8; 64];
        loop {
            match device.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if buf[..n].windows(4).any(|w| w == b"PING") {
                        let _ = device.write_all(b"PONG\n").await;
                    }
                }
            }
        }
    });

    let result = script::run(
        serial_tx,
        tx.subscribe(),
        path.to_string_lossy().into_owned(),
    )
    .await;
    assert!(result.is_ok(), "expected pass, got {result:?}");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn script_expect_timeout_fails() {
    let path = temp_link();
    std::fs::write(&path, "expect 300 NEVERLAND\n").unwrap();

    // Keep the device end alive but silent, so the expect simply times out.
    let (_device, tx, serial_tx) = start_bridge();
    let result = script::run(
        serial_tx,
        tx.subscribe(),
        path.to_string_lossy().into_owned(),
    )
    .await;
    assert!(result.is_err(), "expected timeout failure, got {result:?}");
    let _ = std::fs::remove_file(&path);
}
