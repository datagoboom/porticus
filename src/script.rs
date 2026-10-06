//! A tiny send/expect script runner for device regression checks. Runs against
//! the live bridge and exits nonzero on the first failed expectation, so it
//! drops straight into CI.
//!
//! Script grammar (`#` comments and blank lines ignored):
//!
//! ```text
//! send <text>            send UTF-8 text (escapes: \n \r \t \0 \\ \xHH)
//! send-hex <hex>         send raw bytes ("DE AD BE EF" or "deadbeef")
//! expect <substr>        wait up to 2000ms until received data contains <substr>
//! expect <ms> <substr>   wait up to <ms>
//! delay <ms>             sleep
//! ```

use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tokio::time::{timeout, Instant};
use tracing::info;

use crate::capture::from_hex;

const DEFAULT_EXPECT_MS: u64 = 2000;

/// Executes the script at `path`. `serial_tx` sends to the device; `rx` observes
/// device output. Returns `Err` with a human-readable reason on the first
/// failure.
pub async fn run(
    serial_tx: mpsc::Sender<Vec<u8>>,
    mut rx: broadcast::Receiver<Vec<u8>>,
    path: String,
) -> Result<(), String> {
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("cannot read {path}: {e}"))?;

    let mut buf: Vec<u8> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let lineno = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (cmd, rest) = match line.split_once(char::is_whitespace) {
            Some((c, r)) => (c, r.trim()),
            None => (line, ""),
        };

        match cmd {
            "send" => {
                send(&serial_tx, unescape(rest)).await?;
            }
            "send-hex" => {
                let bytes = from_hex(&rest.replace([' ', ','], ""))
                    .ok_or_else(|| format!("line {lineno}: invalid hex"))?;
                send(&serial_tx, bytes).await?;
            }
            "delay" => {
                let ms: u64 = rest
                    .parse()
                    .map_err(|_| format!("line {lineno}: invalid delay {rest:?}"))?;
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
            "expect" => {
                let (to_ms, needle) = parse_expect(rest);
                if needle.is_empty() {
                    return Err(format!("line {lineno}: expect needs a pattern"));
                }
                wait_for(
                    &mut rx,
                    &mut buf,
                    needle.as_bytes(),
                    Duration::from_millis(to_ms),
                )
                .await
                .map_err(|_| format!("line {lineno}: timeout waiting for {needle:?}"))?;
                info!("expect ok: {needle:?}");
            }
            other => return Err(format!("line {lineno}: unknown command {other:?}")),
        }
    }
    Ok(())
}

async fn send(serial_tx: &mpsc::Sender<Vec<u8>>, bytes: Vec<u8>) -> Result<(), String> {
    serial_tx
        .send(bytes)
        .await
        .map_err(|_| "device write channel closed".to_string())
}

/// Split an `expect` argument into (timeout_ms, needle). A leading all-digits
/// token is taken as the timeout; otherwise the default applies.
fn parse_expect(rest: &str) -> (u64, String) {
    if let Some((head, tail)) = rest.split_once(char::is_whitespace) {
        if !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(ms) = head.parse() {
                return (ms, tail.trim().to_string());
            }
        }
    }
    (DEFAULT_EXPECT_MS, rest.to_string())
}

async fn wait_for(
    rx: &mut broadcast::Receiver<Vec<u8>>,
    buf: &mut Vec<u8>,
    needle: &[u8],
    to: Duration,
) -> Result<(), ()> {
    if contains(buf, needle) {
        return Ok(());
    }
    let deadline = Instant::now() + to;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(());
        }
        match timeout(remaining, rx.recv()).await {
            Ok(Ok(data)) => {
                buf.extend_from_slice(&data);
                if contains(buf, needle) {
                    return Ok(());
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {} // keep waiting
            Ok(Err(broadcast::error::RecvError::Closed)) => return Err(()),
            Err(_) => return Err(()), // timed out
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}

/// C-style escape expansion for `send`.
fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            match b[i + 1] {
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 2;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                }
                b'0' => {
                    out.push(0);
                    i += 2;
                }
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                }
                b'x' if i + 4 <= b.len() => match from_hex(&s[i + 2..i + 4]) {
                    Some(v) => {
                        out.extend(v);
                        i += 4;
                    }
                    None => {
                        out.push(b'\\');
                        i += 1;
                    }
                },
                _ => {
                    out.push(b'\\');
                    i += 1;
                }
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_handles_c_escapes() {
        assert_eq!(unescape("AT"), b"AT");
        assert_eq!(unescape("AT\\r\\n"), b"AT\r\n");
        assert_eq!(unescape("\\x41\\x42"), b"AB");
        assert_eq!(unescape("a\\0b"), vec![b'a', 0, b'b']);
        assert_eq!(unescape("keep\\z"), b"keep\\z"); // unknown escape kept literal
    }

    #[test]
    fn parse_expect_reads_optional_timeout() {
        assert_eq!(parse_expect("500 OK"), (500, "OK".to_string()));
        assert_eq!(parse_expect("OK"), (DEFAULT_EXPECT_MS, "OK".to_string()));
        assert_eq!(
            parse_expect("v1.2 ready"),
            (DEFAULT_EXPECT_MS, "v1.2 ready".to_string())
        );
    }

    #[test]
    fn contains_matches_substring() {
        assert!(contains(b"hello world", b"o w"));
        assert!(!contains(b"hello", b"xyz"));
        assert!(contains(b"anything", b""));
    }
}
