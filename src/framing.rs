//! Optional message framing between the raw serial byte stream and WebSocket
//! clients.
//!
//! Serial is a byte stream with no inherent message boundaries, so a single read
//! can split one logical message across two reads, or glue two together. A
//! [`Framing`] turns that stream into discrete messages in both directions:
//!
//! - **decode** (device → clients): accumulate bytes and emit one message per
//!   complete frame, buffering any partial frame until the rest arrives.
//! - **encode** (clients → device): wrap one outgoing message in the wire format.
//!
//! [`Framing::Raw`] is a passthrough (one message per serial read, no buffering)
//! and the default, preserving the original bridge behavior.

use std::fmt;
use std::str::FromStr;

// SLIP (RFC 1055) special bytes.
const SLIP_END: u8 = 0xC0;
const SLIP_ESC: u8 = 0xDB;
const SLIP_ESC_END: u8 = 0xDC;
const SLIP_ESC_ESC: u8 = 0xDD;

/// How the serial byte stream is split into messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Framing {
    /// No framing: each serial read is forwarded verbatim and outgoing messages
    /// are written as-is. The default — identical to the original bridge.
    #[default]
    Raw,
    /// Newline-delimited text. Splits on `\n` (a trailing `\r` is trimmed) and
    /// emits one message per line without the terminator; encoding appends `\n`.
    Line,
    /// COBS framing (Consistent Overhead Byte Stuffing), zero-delimited. The
    /// payload may contain any bytes; frames are separated by `0x00`.
    Cobs,
    /// SLIP framing (RFC 1055), delimited by `0xC0` with `0xDB` escaping.
    Slip,
}

impl fmt::Display for Framing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Framing::Raw => "raw",
            Framing::Line => "line",
            Framing::Cobs => "cobs",
            Framing::Slip => "slip",
        })
    }
}

impl FromStr for Framing {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "raw" => Ok(Framing::Raw),
            "line" => Ok(Framing::Line),
            "cobs" => Ok(Framing::Cobs),
            "slip" => Ok(Framing::Slip),
            other => Err(format!("unknown framing: {other}")),
        }
    }
}

/// Stateful framer: owns the partial-frame buffer across reads. Create one per
/// open serial connection (so a half-received frame is discarded on reconnect).
pub struct Framer {
    mode: Framing,
    buf: Vec<u8>,
    slip_esc: bool,
}

impl Framer {
    pub fn new(mode: Framing) -> Self {
        Self {
            mode,
            buf: Vec::new(),
            slip_esc: false,
        }
    }

    /// Feed raw bytes read from the device; returns any complete messages. A
    /// partial frame is retained internally and completed by a later call.
    pub fn decode(&mut self, input: &[u8]) -> Vec<Vec<u8>> {
        match self.mode {
            Framing::Raw => {
                if input.is_empty() {
                    Vec::new()
                } else {
                    vec![input.to_vec()]
                }
            }
            Framing::Line => self.decode_line(input),
            Framing::Cobs => self.decode_delimited(input, 0x00, decode_cobs),
            Framing::Slip => self.decode_slip(input),
        }
    }

    /// Wrap one outgoing message into its on-the-wire form.
    pub fn encode(&self, msg: &[u8]) -> Vec<u8> {
        match self.mode {
            Framing::Raw => msg.to_vec(),
            Framing::Line => {
                let mut v = msg.to_vec();
                v.push(b'\n');
                v
            }
            Framing::Cobs => {
                let mut v = encode_cobs(msg);
                v.push(0x00);
                v
            }
            Framing::Slip => encode_slip(msg),
        }
    }

    fn decode_line(&mut self, input: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for &b in input {
            if b == b'\n' {
                let mut line = std::mem::take(&mut self.buf);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                out.push(line);
            } else {
                self.buf.push(b);
            }
        }
        out
    }

    /// Accumulate until `delim`, then decode each complete frame with `decode_fn`.
    /// Empty frames (back-to-back delimiters) are skipped.
    fn decode_delimited(
        &mut self,
        input: &[u8],
        delim: u8,
        decode_fn: fn(&[u8]) -> Vec<u8>,
    ) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for &b in input {
            if b == delim {
                if !self.buf.is_empty() {
                    out.push(decode_fn(&self.buf));
                    self.buf.clear();
                }
            } else {
                self.buf.push(b);
            }
        }
        out
    }

    fn decode_slip(&mut self, input: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for &b in input {
            if self.slip_esc {
                match b {
                    SLIP_ESC_END => self.buf.push(SLIP_END),
                    SLIP_ESC_ESC => self.buf.push(SLIP_ESC),
                    other => self.buf.push(other), // protocol violation: keep byte
                }
                self.slip_esc = false;
            } else {
                match b {
                    SLIP_END => {
                        if !self.buf.is_empty() {
                            out.push(std::mem::take(&mut self.buf));
                        }
                    }
                    SLIP_ESC => self.slip_esc = true,
                    other => self.buf.push(other),
                }
            }
        }
        out
    }
}

/// COBS-encode `data` (no trailing delimiter; the caller appends `0x00`).
fn encode_cobs(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 254 + 2);
    let mut code_idx = out.len();
    out.push(0); // placeholder for the first code byte
    let mut code: u8 = 1;
    for &b in data {
        if b == 0 {
            out[code_idx] = code;
            code_idx = out.len();
            out.push(0);
            code = 1;
        } else {
            out.push(b);
            code += 1;
            if code == 0xFF {
                out[code_idx] = code;
                code_idx = out.len();
                out.push(0);
                code = 1;
            }
        }
    }
    out[code_idx] = code;
    out
}

/// COBS-decode one frame (delimiter already stripped). Lenient on malformed
/// input: never panics, stops at the end of the slice.
fn decode_cobs(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let code = data[i] as usize;
        i += 1;
        if code == 0 {
            break; // invalid: a code byte is never zero
        }
        for _ in 0..code - 1 {
            if i < data.len() {
                out.push(data[i]);
                i += 1;
            }
        }
        // Every block except a full 0xFF run and the final block implies a zero.
        if code != 0xFF && i < data.len() {
            out.push(0);
        }
    }
    out
}

/// SLIP-encode `data`, including the trailing `END` delimiter.
fn encode_slip(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 2);
    for &b in data {
        match b {
            SLIP_END => {
                out.push(SLIP_ESC);
                out.push(SLIP_ESC_END);
            }
            SLIP_ESC => {
                out.push(SLIP_ESC);
                out.push(SLIP_ESC_ESC);
            }
            other => out.push(other),
        }
    }
    out.push(SLIP_END);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `input` to a framer one byte at a time, which is the worst case for
    /// partial-frame handling, and returns everything decoded.
    fn decode_byte_by_byte(mode: Framing, input: &[u8]) -> Vec<Vec<u8>> {
        let mut framer = Framer::new(mode);
        let mut out = Vec::new();
        for &b in input {
            out.extend(framer.decode(&[b]));
        }
        out
    }

    #[test]
    fn raw_forwards_each_read_verbatim() {
        let mut f = Framer::new(Framing::Raw);
        assert_eq!(f.decode(b"abc"), vec![b"abc".to_vec()]);
        assert_eq!(f.decode(b""), Vec::<Vec<u8>>::new());
        assert_eq!(f.encode(b"xyz"), b"xyz".to_vec());
    }

    #[test]
    fn line_splits_on_newline_and_trims_cr() {
        let mut f = Framer::new(Framing::Line);
        // Split across two reads; CRLF and bare LF both yield clean lines.
        assert_eq!(f.decode(b"hel"), Vec::<Vec<u8>>::new());
        assert_eq!(f.decode(b"lo\r\nwor"), vec![b"hello".to_vec()]);
        assert_eq!(f.decode(b"ld\n"), vec![b"world".to_vec()]);
        assert_eq!(f.encode(b"AT"), b"AT\n".to_vec());
    }

    #[test]
    fn line_byte_by_byte_matches() {
        let out = decode_byte_by_byte(Framing::Line, b"one\ntwo\nthree\n");
        assert_eq!(
            out,
            vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
        );
    }

    #[test]
    fn cobs_round_trips_including_zeros() {
        for case in [
            &b""[..],
            &b"\x00"[..],
            &b"\x11\x22\x00\x33"[..],
            &b"\x00\x00\x00"[..],
            &[1u8; 300][..], // forces a 0xFF code-block boundary
            &b"hello world"[..],
        ] {
            let mut f = Framer::new(Framing::Cobs);
            let wire = f.encode(case); // includes the 0x00 delimiter
            assert_eq!(wire.last(), Some(&0u8), "encoded frame must end in delim");
            assert!(
                wire[..wire.len() - 1].iter().all(|&b| b != 0),
                "COBS body must contain no zero bytes"
            );
            let decoded = f.decode(&wire);
            assert_eq!(
                decoded,
                vec![case.to_vec()],
                "round-trip failed for {case:?}"
            );
        }
    }

    #[test]
    fn cobs_reassembles_across_reads() {
        let mut f = Framer::new(Framing::Cobs);
        let wire = Framer::new(Framing::Cobs).encode(b"\x11\x00\x22");
        // Split the wire frame at every point; the whole frame arrives only once
        // the delimiter does.
        let (a, b) = wire.split_at(2);
        assert_eq!(f.decode(a), Vec::<Vec<u8>>::new());
        assert_eq!(f.decode(b), vec![b"\x11\x00\x22".to_vec()]);
    }

    #[test]
    fn slip_round_trips_including_special_bytes() {
        for case in [
            &b""[..],
            &[SLIP_END][..],
            &[SLIP_ESC][..],
            &[SLIP_END, SLIP_ESC, 0x01, SLIP_END][..],
            &b"packet"[..],
        ] {
            let decoded =
                decode_byte_by_byte(Framing::Slip, &Framer::new(Framing::Slip).encode(case));
            if case.is_empty() {
                // An empty payload encodes to a lone END and decodes to nothing.
                assert!(decoded.is_empty());
            } else {
                assert_eq!(
                    decoded,
                    vec![case.to_vec()],
                    "round-trip failed for {case:?}"
                );
            }
        }
    }

    #[test]
    fn framing_parses_from_str() {
        assert_eq!("LINE".parse::<Framing>().unwrap(), Framing::Line);
        assert_eq!("cobs".parse::<Framing>().unwrap(), Framing::Cobs);
        assert!("nope".parse::<Framing>().is_err());
    }
}
