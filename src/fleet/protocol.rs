//! The agent↔hub multiplexing protocol.
//!
//! One WebSocket per agent carries every device it owns:
//! - the agent's first message is a **HELLO** (a JSON text frame) announcing the
//!   node name and its device list, in index order;
//! - every subsequent message is a **DATA** binary frame tagging a payload with
//!   the device index from HELLO. Direction is implied by the sender
//!   (agent→hub = device output, hub→agent = bytes to write to the device).

use serde::{Deserialize, Serialize};

/// Opcode byte prefixing a binary DATA frame.
const DATA: u8 = 0x02;

/// Agent announcement: node identity and the devices it exposes (index order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub node: String,
    pub devices: Vec<DeviceInfo>,
}

/// One device as advertised to the hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub alias: String,
    pub baud: u32,
    pub framing: String,
}

impl Hello {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Option<Hello> {
        serde_json::from_str(s).ok()
    }
}

/// Encode a DATA frame: `[DATA][u16 device index BE][payload]`.
pub fn encode_data(device: u16, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(3 + payload.len());
    v.push(DATA);
    v.extend_from_slice(&device.to_be_bytes());
    v.extend_from_slice(payload);
    v
}

/// Decode a DATA frame into (device index, payload). Returns None if the frame
/// is too short or has an unknown opcode.
pub fn decode_data(frame: &[u8]) -> Option<(u16, &[u8])> {
    if frame.len() < 3 || frame[0] != DATA {
        return None;
    }
    let device = u16::from_be_bytes([frame[1], frame[2]]);
    Some((device, &frame[3..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_frame_round_trips() {
        let frame = encode_data(7, b"hello");
        assert_eq!(decode_data(&frame), Some((7u16, &b"hello"[..])));
        assert_eq!(decode_data(&frame[..2]), None); // too short
        assert_eq!(decode_data(&[0x09, 0, 0]), None); // wrong opcode
    }

    #[test]
    fn hello_round_trips() {
        let hello = Hello {
            node: "pi-1".into(),
            devices: vec![DeviceInfo {
                alias: "uno".into(),
                baud: 115200,
                framing: "line".into(),
            }],
        };
        assert_eq!(Hello::from_json(&hello.to_json()), Some(hello));
    }
}
