//! Agent mode: open the allowlisted serial ports and multiplex them over a
//! single outbound WebSocket to the hub. The per-port serial tasks are
//! persistent (they keep their own reconnect-on-replug behavior); only the hub
//! link re-dials, with its own exponential backoff.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::{client_async, connect_async, tungstenite::Message, WebSocketStream};
use tracing::{info, warn};

use crate::config::PorticusConfig;
use crate::fleet::config::AgentConfig;
use crate::fleet::protocol::{decode_data, encode_data, DeviceInfo, Hello};
use crate::fleet::tls;
use crate::serial;

const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// One exposed device: its output fanout and its write channel to the serial port.
struct Device {
    info: DeviceInfo,
    rx: broadcast::Sender<Vec<u8>>,
    tx: mpsc::Sender<Vec<u8>>,
}

/// Runs the agent forever: starts the serial tasks, then keeps a connection to
/// the hub alive across drops.
pub async fn run(cfg: AgentConfig) {
    let mut devices: Vec<Device> = Vec::new();
    for port in &cfg.ports {
        let (rx, _) = broadcast::channel::<Vec<u8>>(256);
        let (tx, serial_rx) = mpsc::channel::<Vec<u8>>(64);
        let pcfg = PorticusConfig {
            serial_port: port.path.clone(),
            baud_rate: port.baud,
            framing: port.framing(),
            ..PorticusConfig::default()
        };
        tokio::spawn(serial::run(pcfg, rx.clone(), serial_rx, None));
        info!(node = %cfg.node, device = %port.alias(), path = %port.path, "exposing serial port");
        devices.push(Device {
            info: DeviceInfo {
                alias: port.alias(),
                baud: port.baud,
                framing: port.framing().to_string(),
            },
            rx,
            tx,
        });
    }

    let mut backoff = Duration::from_secs(1);
    loop {
        match serve(&cfg, &devices).await {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(e) => warn!("hub link: {e}"),
        }
        info!("reconnecting to hub in {}s", backoff.as_secs());
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// One hub connection: HELLO, then pump device output up and device writes down
/// until the link drops.
async fn serve(cfg: &AgentConfig, devices: &[Device]) -> Result<(), String> {
    match &cfg.tls {
        Some(t) => {
            let ws = connect_tls(cfg, t).await?;
            info!(hub = %cfg.hub, node = %cfg.node, "connected to hub (mTLS)");
            run_link(cfg, devices, ws).await
        }
        None => {
            let (ws, _) = connect_async(&cfg.hub)
                .await
                .map_err(|e| format!("connect {}: {e}", cfg.hub))?;
            info!(hub = %cfg.hub, node = %cfg.node, "connected to hub");
            run_link(cfg, devices, ws).await
        }
    }
}

/// Dial the hub over mutual TLS and complete the WebSocket handshake.
async fn connect_tls(
    cfg: &AgentConfig,
    t: &crate::fleet::config::TlsConfig,
) -> Result<WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>, String> {
    let client_config = tls::client_config(&t.ca, &t.cert, &t.key)?;

    let rest = cfg
        .hub
        .strip_prefix("wss://")
        .or_else(|| cfg.hub.strip_prefix("ws://"))
        .unwrap_or(&cfg.hub);
    let hostport = rest.split('/').next().unwrap_or(rest);
    let (host, port) = hostport
        .rsplit_once(':')
        .ok_or_else(|| format!("hub url needs host:port: {}", cfg.hub))?;
    let port: u16 = port
        .parse()
        .map_err(|_| format!("bad port in {}", cfg.hub))?;

    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("connect {host}:{port}: {e}"))?;
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| format!("bad server name {host}: {e}"))?;
    let tls_stream = TlsConnector::from(client_config)
        .connect(server_name, tcp)
        .await
        .map_err(|e| format!("tls: {e}"))?;
    let (ws, _) = client_async(&cfg.hub, tls_stream)
        .await
        .map_err(|e| format!("ws handshake: {e}"))?;
    Ok(ws)
}

/// Pump the HELLO, device output, and device writes over one hub link until it
/// drops. Generic over the transport (plain TCP or TLS).
async fn run_link<S>(
    cfg: &AgentConfig,
    devices: &[Device],
    ws: WebSocketStream<S>,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut stream) = ws.split();

    let hello = Hello {
        node: cfg.node.clone(),
        devices: devices.iter().map(|d| d.info.clone()).collect(),
    };
    sink.send(Message::text(hello.to_json()))
        .await
        .map_err(|e| format!("hello: {e}"))?;

    // All outbound frames go through one writer task (a single owner of the sink).
    let (wtx, mut wrx) = mpsc::channel::<Message>(256);
    let writer = tokio::spawn(async move {
        while let Some(msg) = wrx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // One forwarder per device: device output → DATA frames to the hub.
    let mut forwarders: Vec<JoinHandle<()>> = Vec::new();
    for (idx, dev) in devices.iter().enumerate() {
        let mut sub = dev.rx.subscribe();
        let wtx = wtx.clone();
        forwarders.push(tokio::spawn(async move {
            loop {
                match sub.recv().await {
                    Ok(bytes) => {
                        let frame = Message::binary(encode_data(idx as u16, &bytes));
                        if wtx.send(frame).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }));
    }

    // Reader: hub → agent DATA frames become device writes.
    let result = loop {
        match stream.next().await {
            Some(Ok(Message::Binary(frame))) => {
                if let Some((idx, payload)) = decode_data(frame.as_ref()) {
                    if let Some(dev) = devices.get(idx as usize) {
                        let _ = dev.tx.send(payload.to_vec()).await;
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => break Ok(()),
            Some(Ok(_)) => {} // ignore text/ping/pong
            Some(Err(e)) => break Err(format!("stream: {e}")),
        }
    };

    for f in forwarders {
        f.abort();
    }
    drop(wtx);
    writer.abort();
    result
}
