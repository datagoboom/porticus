#![cfg(unix)]

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tokio_serial::SerialStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use porticus::{serial, websocket};

const WAIT: Duration = Duration::from_secs(5);

/// Spawns the serial bridge on the slave end of a PTY pair and returns the
/// master end (acting as the "device") plus the bridge's channel endpoints.
fn start_bridge() -> (
    SerialStream,
    broadcast::Sender<Vec<u8>>,
    mpsc::Sender<Vec<u8>>,
) {
    let (device, port) = SerialStream::pair().expect("failed to create PTY pair");
    let (tx, _) = broadcast::channel(16);
    let (serial_tx, mut serial_rx) = mpsc::channel(16);
    let bridge_tx = tx.clone();
    tokio::spawn(async move {
        let _ = serial::bridge(port, &bridge_tx, &mut serial_rx, 1024).await;
    });
    (device, tx, serial_tx)
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
