//! A tiny static HTTP server for the browser console. It serves exactly two
//! things — the embedded single-page console and a `/info` descriptor — so it is
//! hand-rolled rather than pulling in a web framework, keeping the dependency
//! footprint unchanged. It is entirely separate from the WebSocket listener;
//! existing ws clients are untouched whether or not the console is enabled.

use std::borrow::Cow;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::warn;

use crate::config::PorticusConfig;

/// The whole console, inlined (HTML + CSS + JS) at build time. Built from
/// `client/` with `npm run build`; the output is committed so `cargo build`
/// never depends on the node toolchain.
pub const CONSOLE_HTML: &str = include_str!("../client/dist/index.html");

/// Serves the console until the process exits. A failed accept is logged and
/// skipped (with a short backoff) rather than taking the server down.
pub async fn run(listener: TcpListener, config: PorticusConfig) {
    let info = info_json(&config);
    loop {
        let mut stream = match listener.accept().await {
            Ok((stream, _addr)) => stream,
            Err(e) => {
                warn!("console accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let info = info.clone();
        tokio::spawn(async move {
            if let Err(e) = serve(&mut stream, &info).await {
                warn!("console request error: {e}");
            }
        });
    }
}

async fn serve(stream: &mut TcpStream, info: &str) -> std::io::Result<()> {
    // The request line is all we need; read one chunk and parse the path out of
    // "GET /path HTTP/1.1". Bodies are irrelevant for these GET routes.
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&buf[..n]);
    let path = request.split_whitespace().nth(1).unwrap_or("/");

    let (status, content_type, body): (&str, &str, Cow<str>) = match path {
        "/" | "/index.html" => (
            "200 OK",
            "text/html; charset=utf-8",
            Cow::Borrowed(CONSOLE_HTML),
        ),
        "/info" => ("200 OK", "application/json", Cow::Borrowed(info)),
        _ => (
            "404 Not Found",
            "text/plain; charset=utf-8",
            Cow::Borrowed("not found"),
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
    stream.flush().await
}

/// Hand-built JSON (no serde dependency) describing the bridge for the console.
fn info_json(c: &PorticusConfig) -> String {
    format!(
        "{{\"serial_port\":{},\"baud_rate\":{},\"framing\":\"{}\",\"websocket_port\":{},\"websocket_host\":{}}}",
        json_string(&c.serial_port),
        c.baud_rate,
        c.framing,
        c.websocket_port,
        json_string(&c.websocket_host),
    )
}

/// Minimal JSON string escaping — enough for paths/hostnames (quotes, backslash,
/// control chars).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
