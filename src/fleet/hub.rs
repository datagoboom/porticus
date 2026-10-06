//! Hub mode: aggregate many agents and re-expose their devices to clients.
//!
//! Two listeners:
//! - the **agent listener** accepts outbound agent connections, reads their
//!   HELLO, and keeps a `node → devices` registry;
//! - the **client listener** serves clients on one port: `GET /fleet` (registry
//!   JSON) and `ws /<node>/<device>` (a live stream of that device, with writes
//!   routed back to the owning agent). HTTP vs WebSocket is told apart by peeking
//!   the request head.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rustls::ServerConfig;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};
use tracing::{info, warn};

use crate::fleet::protocol::{decode_data, encode_data, DeviceInfo, Hello};
use crate::fleet::tls;

/// A registered agent and the plumbing to reach its devices.
pub struct Node {
    name: String,
    devices: Vec<DeviceInfo>,
    /// Per-device fanout of device output to connected clients.
    rx: Vec<broadcast::Sender<Vec<u8>>>,
    /// Frames to write back to the agent (device writes).
    to_agent: mpsc::Sender<Message>,
}

/// User-assigned, hub-persisted device identity (keyed by "node/alias").
#[derive(Clone, Default, Serialize, Deserialize)]
struct DeviceMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    nick: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}

#[derive(Clone, Default)]
pub struct Hub {
    nodes: Arc<Mutex<HashMap<String, Arc<Node>>>>,
    meta: Arc<Mutex<HashMap<String, DeviceMeta>>>,
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load persisted device nicknames/colors from the hub dir.
    pub fn load_meta(&self) {
        if let Ok(dir) = tls::hub_dir() {
            if let Ok(text) = std::fs::read_to_string(dir.join("devices.json")) {
                if let Ok(map) = serde_json::from_str::<HashMap<String, DeviceMeta>>(&text) {
                    *self.meta.lock().unwrap() = map;
                }
            }
        }
    }

    fn save_meta(&self) {
        let Ok(dir) = tls::hub_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let snapshot = self.meta.lock().unwrap().clone();
        if let Ok(text) = serde_json::to_string_pretty(&snapshot) {
            let _ = std::fs::write(dir.join("devices.json"), text);
        }
    }

    fn find(&self, node: &str, device: &str) -> Option<(Arc<Node>, usize)> {
        let map = self.nodes.lock().unwrap();
        let n = map.get(node)?.clone();
        let idx = n.devices.iter().position(|d| d.alias == device)?;
        Some((n, idx))
    }

    fn fleet_json(&self) -> String {
        #[derive(Serialize)]
        struct FleetView {
            nodes: Vec<NodeView>,
        }
        #[derive(Serialize)]
        struct NodeView {
            node: String,
            devices: Vec<DeviceView>,
        }
        #[derive(Serialize)]
        struct DeviceView {
            alias: String,
            baud: u32,
            framing: String,
            nick: Option<String>,
            color: Option<String>,
        }
        let nodes_map = self.nodes.lock().unwrap();
        let meta = self.meta.lock().unwrap();
        let nodes: Vec<NodeView> = nodes_map
            .values()
            .map(|n| NodeView {
                node: n.name.clone(),
                devices: n
                    .devices
                    .iter()
                    .map(|d| {
                        let m = meta.get(&format!("{}/{}", n.name, d.alias));
                        DeviceView {
                            alias: d.alias.clone(),
                            baud: d.baud,
                            framing: d.framing.clone(),
                            nick: m.and_then(|m| m.nick.clone()),
                            color: m.and_then(|m| m.color.clone()),
                        }
                    })
                    .collect(),
            })
            .collect();
        serde_json::to_string(&FleetView { nodes }).unwrap_or_else(|_| "{\"nodes\":[]}".into())
    }

    /// Apply a nickname/color update from POST /meta and persist it. Empty
    /// strings clear the field; a device with neither is dropped from the store.
    fn set_meta(&self, node: &str, alias: &str, nick: Option<String>, color: Option<String>) {
        let key = format!("{node}/{alias}");
        let clean = |s: Option<String>| s.filter(|v| !v.trim().is_empty());
        {
            let mut map = self.meta.lock().unwrap();
            let nick = clean(nick);
            let color = clean(color);
            if nick.is_none() && color.is_none() {
                map.remove(&key);
            } else {
                map.insert(key, DeviceMeta { nick, color });
            }
        }
        self.save_meta();
    }

    // ----- agent side -----

    /// Accepts agent connections forever. With `tls` set, each connection must
    /// complete mutual TLS and present an enrolled client certificate.
    pub async fn run_agent_listener(self, listener: TcpListener, tls: Option<Arc<ServerConfig>>) {
        loop {
            let (stream, addr) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    warn!("agent accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let hub = self.clone();
            let tls = tls.clone();
            tokio::spawn(async move {
                if let Err(e) = hub.accept_agent(stream, tls).await {
                    warn!(%addr, "agent connection: {e}");
                }
            });
        }
    }

    /// Establish the transport (plain or mTLS) and hand off to the generic
    /// per-agent handler.
    async fn accept_agent(
        &self,
        tcp: TcpStream,
        tls: Option<Arc<ServerConfig>>,
    ) -> Result<(), String> {
        match tls {
            Some(cfg) => {
                let tls_stream = TlsAcceptor::from(cfg)
                    .accept(tcp)
                    .await
                    .map_err(|e| format!("tls handshake: {e}"))?;
                // Gate on the client certificate fingerprint (revocation).
                let fp = tls_stream
                    .get_ref()
                    .1
                    .peer_certificates()
                    .and_then(|c| c.first())
                    .map(|c| tls::fingerprint_hex(c.as_ref()));
                match fp {
                    Some(fp) if tls::load_enrolled().contains(&fp) => {
                        let ws = accept_async(tls_stream)
                            .await
                            .map_err(|e| format!("handshake: {e}"))?;
                        self.handle_agent_ws(ws).await
                    }
                    Some(fp) => Err(format!("rejected: certificate {fp} is not enrolled")),
                    None => Err("rejected: no client certificate".into()),
                }
            }
            None => {
                let ws = accept_async(tcp)
                    .await
                    .map_err(|e| format!("handshake: {e}"))?;
                self.handle_agent_ws(ws).await
            }
        }
    }

    async fn handle_agent_ws<S>(&self, ws: WebSocketStream<S>) -> Result<(), String>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (mut sink, mut stream) = ws.split();

        // The first message must be HELLO.
        let hello = match stream.next().await {
            Some(Ok(Message::Text(t))) => Hello::from_json(t.as_str()).ok_or("malformed hello")?,
            _ => return Err("expected hello".into()),
        };
        let name = hello.node.clone();
        info!(node = %name, devices = hello.devices.len(), "agent registered");

        let rx: Vec<broadcast::Sender<Vec<u8>>> = hello
            .devices
            .iter()
            .map(|_| broadcast::channel::<Vec<u8>>(256).0)
            .collect();
        let (to_agent, mut agent_rx) = mpsc::channel::<Message>(256);
        let node = Arc::new(Node {
            name: name.clone(),
            devices: hello.devices.clone(),
            rx,
            to_agent,
        });
        self.nodes
            .lock()
            .unwrap()
            .insert(name.clone(), node.clone());

        // Writer task: hub → agent.
        let writer = tokio::spawn(async move {
            while let Some(msg) = agent_rx.recv().await {
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Reader: agent → hub DATA frames fan out to each device's clients.
        let result = loop {
            match stream.next().await {
                Some(Ok(Message::Binary(frame))) => {
                    if let Some((idx, payload)) = decode_data(frame.as_ref()) {
                        if let Some(tx) = node.rx.get(idx as usize) {
                            let _ = tx.send(payload.to_vec());
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break Ok(()),
                Some(Ok(_)) => {}
                Some(Err(e)) => break Err(format!("stream: {e}")),
            }
        };

        // Deregister, but only if this is still the live entry (a reconnect may
        // have replaced us).
        {
            let mut map = self.nodes.lock().unwrap();
            if map.get(&name).is_some_and(|n| Arc::ptr_eq(n, &node)) {
                map.remove(&name);
            }
        }
        writer.abort();
        info!(node = %name, "agent disconnected");
        result
    }

    // ----- client side -----

    /// Serves clients forever (HTTP registry + per-device WebSocket).
    pub async fn run_client_listener(self, listener: TcpListener) {
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _addr)) => stream,
                Err(e) => {
                    warn!("client accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let hub = self.clone();
            tokio::spawn(async move {
                let _ = hub.handle_client(stream).await;
            });
        }
    }

    async fn handle_client(&self, stream: TcpStream) -> std::io::Result<()> {
        // Peek (don't consume) the request head to tell a WebSocket upgrade from
        // a plain HTTP request, and to read the path.
        let mut head = [0u8; 1024];
        let n = stream.peek(&mut head).await?;
        let text = String::from_utf8_lossy(&head[..n]);
        let mut tokens = text.split_whitespace();
        let method = tokens.next().unwrap_or("GET").to_ascii_uppercase();
        let path = tokens.next().unwrap_or("/").to_string();
        let is_ws = text.to_ascii_lowercase().contains("upgrade: websocket");

        if is_ws {
            self.serve_device_ws(stream, path).await;
            Ok(())
        } else if method == "POST" && path == "/meta" {
            self.handle_meta(stream).await
        } else {
            self.serve_http(stream, &path).await
        }
    }

    /// POST /meta — set a device's nickname/color. Body is JSON
    /// `{"node","alias","nick","color"}` (empty strings clear a field).
    async fn handle_meta(&self, mut stream: TcpStream) -> std::io::Result<()> {
        #[derive(Deserialize)]
        struct MetaReq {
            node: String,
            alias: String,
            #[serde(default)]
            nick: Option<String>,
            #[serde(default)]
            color: Option<String>,
        }

        // Read the full request (headers + body up to Content-Length).
        let mut buf = Vec::new();
        let mut tmp = [0u8; 2048];
        let mut ok = false;
        loop {
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(split) = find_headers_end(&buf) {
                let clen = content_length(&buf[..split]).unwrap_or(0);
                if buf.len() >= split + clen {
                    let body = &buf[split..split + clen];
                    if let Ok(req) = serde_json::from_slice::<MetaReq>(body) {
                        self.set_meta(&req.node, &req.alias, req.nick, req.color);
                        info!(node = %req.node, alias = %req.alias, "device meta updated");
                        ok = true;
                    }
                    break;
                }
            }
            if buf.len() > 64 * 1024 {
                break;
            }
        }

        let (status, body) = if ok {
            ("200 OK", "{\"ok\":true}")
        } else {
            ("400 Bad Request", "{\"ok\":false}")
        };
        let response = format!(
            "HTTP/1.1 {status}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len(),
        );
        stream.write_all(response.as_bytes()).await?;
        stream.flush().await?;
        stream.shutdown().await
    }

    async fn serve_http(&self, mut stream: TcpStream, path: &str) -> std::io::Result<()> {
        // We only peeked the request head; consume the request now so closing the
        // socket sends a clean FIN instead of an RST on unread bytes.
        let mut scratch = [0u8; 2048];
        let _ = stream.read(&mut scratch).await;

        let (status, content_type, body) = match path {
            "/fleet" => ("200 OK", "application/json", self.fleet_json()),
            "/" | "/index.html" => (
                "200 OK",
                "text/html; charset=utf-8",
                crate::http::CONSOLE_HTML.to_string(),
            ),
            _ => (
                "404 Not Found",
                "text/plain; charset=utf-8",
                "not found".to_string(),
            ),
        };
        let response = format!(
            "HTTP/1.1 {status}\r\n\
             Content-Type: {content_type}\r\n\
             Content-Length: {}\r\n\
             Cache-Control: no-store\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len(),
        );
        stream.write_all(response.as_bytes()).await?;
        stream.flush().await?;
        stream.shutdown().await
    }

    async fn serve_device_ws(&self, stream: TcpStream, path: String) {
        let ws = match accept_async(stream).await {
            Ok(ws) => ws,
            Err(_) => return,
        };
        // Path is /<node>/<device>.
        let trimmed = path.trim_matches('/');
        let Some((node_name, device)) = trimmed.split_once('/') else {
            return;
        };
        let Some((node, idx)) = self.find(node_name, device) else {
            info!("client requested unknown device /{trimmed}");
            return; // dropping the ws closes it
        };

        let (mut sink, mut stream) = ws.split();
        let mut sub = node.rx[idx].subscribe();

        // Device output → client.
        let to_client = tokio::spawn(async move {
            loop {
                match sub.recv().await {
                    Ok(bytes) => {
                        if sink.send(Message::binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        // Client → device (routed back to the owning agent).
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Binary(b)) => {
                    let _ = node
                        .to_agent
                        .send(Message::binary(encode_data(idx as u16, b.as_ref())))
                        .await;
                }
                Ok(Message::Text(t)) => {
                    let _ = node
                        .to_agent
                        .send(Message::binary(encode_data(idx as u16, t.as_bytes())))
                        .await;
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
        to_client.abort();
    }
}

/// Index just past the `\r\n\r\n` header terminator, if present.
fn find_headers_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

/// Parse the `Content-Length` value from a request's header bytes.
fn content_length(headers: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(headers).ok()?;
    for line in text.lines() {
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            return v.trim().parse().ok();
        }
    }
    None
}
