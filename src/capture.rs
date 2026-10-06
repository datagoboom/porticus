//! Session capture: a simple, greppable record/replay format.
//!
//! Each line is `<elapsed_ms> <dir> <hex>`:
//!
//! ```text
//! 0 rx 48656c6c6f
//! 310 tx 4154
//! ```
//!
//! where `elapsed_ms` is milliseconds since capture start, `dir` is `rx`
//! (device → clients) or `tx` (clients → device), and `hex` is the
//! lowercase-hex payload. `#` lines and blanks are ignored on read.

use std::time::{Duration, Instant};

use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

/// Direction of a captured message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Device → clients.
    Rx,
    /// Clients → device.
    Tx,
}

impl Dir {
    pub fn as_str(self) -> &'static str {
        match self {
            Dir::Rx => "rx",
            Dir::Tx => "tx",
        }
    }
    pub fn parse(s: &str) -> Option<Dir> {
        match s {
            "rx" => Some(Dir::Rx),
            "tx" => Some(Dir::Tx),
            _ => None,
        }
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(nibble(b >> 4));
        s.push(nibble(b & 0x0f));
    }
    s
}

fn nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'a' + (n - 10)) as char,
    }
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let b = s.as_bytes();
    let val = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < b.len() {
        out.push((val(b[i])? << 4) | val(b[i + 1])?);
        i += 2;
    }
    Some(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub at_ms: u64,
    pub dir: Dir,
    pub bytes: Vec<u8>,
}

/// Parse a capture file's text into events, skipping blanks, comments, and any
/// malformed lines.
pub fn parse(text: &str) -> Vec<Event> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(t), Some(d), Some(hex)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let (Ok(at_ms), Some(dir), Some(bytes)) = (t.parse::<u64>(), Dir::parse(d), from_hex(hex))
        else {
            continue;
        };
        out.push(Event { at_ms, dir, bytes });
    }
    out
}

/// Records capture events to `path` until the event channel closes. Timestamps
/// are relative to the first event. Writes are best-effort and never block the
/// bridge (the sender uses `try_send`).
pub async fn record(mut events: mpsc::Receiver<(Dir, Vec<u8>)>, path: String) {
    let file = match File::create(&path).await {
        Ok(f) => f,
        Err(e) => {
            warn!("recording disabled: cannot create {path}: {e}");
            return;
        }
    };
    info!("recording session to {path}");
    let mut w = BufWriter::new(file);
    let start = Instant::now();
    while let Some((dir, bytes)) = events.recv().await {
        let line = format!(
            "{} {} {}\n",
            start.elapsed().as_millis(),
            dir.as_str(),
            to_hex(&bytes)
        );
        if w.write_all(line.as_bytes()).await.is_err() {
            break;
        }
        let _ = w.flush().await;
    }
    let _ = w.flush().await;
}

/// Replays a capture as a virtual device: `rx` events are fed into `tx` at their
/// original timing so connected clients see the recorded stream. Client writes
/// arriving on `write_rx` are discarded. After the last event it keeps draining
/// writes so clients stay connected.
pub async fn replay(
    tx: broadcast::Sender<Vec<u8>>,
    mut write_rx: mpsc::Receiver<Vec<u8>>,
    path: String,
) {
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(t) => t,
        Err(e) => {
            warn!("replay failed: cannot read {path}: {e}");
            return;
        }
    };
    let events: Vec<Event> = parse(&text)
        .into_iter()
        .filter(|e| e.dir == Dir::Rx)
        .collect();
    info!("replaying {} events from {path}", events.len());

    let start = Instant::now();
    let mut idx = 0;
    loop {
        if idx >= events.len() {
            match write_rx.recv().await {
                Some(_) => continue, // discard client writes, stay connected
                None => return,
            }
        }
        let target = Duration::from_millis(events[idx].at_ms);
        let wait = target
            .checked_sub(start.elapsed())
            .unwrap_or(Duration::ZERO);
        tokio::select! {
            _ = tokio::time::sleep(wait) => {
                let _ = tx.send(events[idx].bytes.clone());
                idx += 1;
            }
            msg = write_rx.recv() => {
                if msg.is_none() { return; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        for case in [&b""[..], &b"\x00\xff\x10"[..], &b"hello"[..]] {
            assert_eq!(from_hex(&to_hex(case)).as_deref(), Some(case));
        }
        assert_eq!(from_hex("abc"), None); // odd length
        assert_eq!(from_hex("zz"), None); // non-hex
    }

    #[test]
    fn parse_skips_junk_and_reads_events() {
        let text = "# a comment\n\n0 rx 4142\n10 tx 4154\nbroken line\n20 rx zz\n30 rx 00\n";
        let events = parse(text);
        assert_eq!(
            events,
            vec![
                Event {
                    at_ms: 0,
                    dir: Dir::Rx,
                    bytes: b"AB".to_vec()
                },
                Event {
                    at_ms: 10,
                    dir: Dir::Tx,
                    bytes: b"AT".to_vec()
                },
                Event {
                    at_ms: 30,
                    dir: Dir::Rx,
                    bytes: vec![0]
                },
            ]
        );
    }
}
