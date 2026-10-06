//! Agent configuration (TOML). Lists the hub to dial and the allowlist of serial
//! ports this node exposes — an agent only ever bridges ports named here.
//!
//! ```toml
//! hub  = "ws://server:9000"
//! node = "pi-bench-1"
//!
//! [[port]]
//! path   = "/dev/ttyACM0"
//! baud   = 115200
//! alias  = "uno"
//! framing = "line"
//! ```

use std::path::Path;

use serde::Deserialize;

use crate::framing::Framing;

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    /// Hub URL to dial out to (ws://… in Phase 1, wss://… once mTLS lands).
    pub hub: String,
    /// This node's name, used to address its devices at the hub.
    pub node: String,
    /// Allowlisted serial ports. TOML key is `[[port]]`.
    #[serde(default, rename = "port")]
    pub ports: Vec<PortConfig>,
    /// Optional mutual-TLS material. When present, the agent dials the hub over
    /// wss and presents its client certificate.
    #[serde(default)]
    pub tls: Option<TlsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    /// CA certificate that signs the hub (and this agent).
    pub ca: String,
    /// This agent's client certificate.
    pub cert: String,
    /// This agent's private key.
    pub key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PortConfig {
    pub path: String,
    #[serde(default = "default_baud")]
    pub baud: u32,
    /// Short name for the device at the hub; defaults to the path's file name.
    pub alias: Option<String>,
    /// Framing mode name (raw/line/cobs/slip); defaults to raw.
    pub framing: Option<String>,
}

fn default_baud() -> u32 {
    9600
}

impl PortConfig {
    pub fn alias(&self) -> String {
        self.alias.clone().unwrap_or_else(|| {
            Path::new(&self.path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&self.path)
                .to_string()
        })
    }

    pub fn framing(&self) -> Framing {
        self.framing
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(Framing::Raw)
    }
}

/// Load and parse an agent config file.
pub fn load(path: &str) -> Result<AgentConfig, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let cfg: AgentConfig =
        toml::from_str(&text).map_err(|e| format!("invalid config {path}: {e}"))?;
    if cfg.ports.is_empty() {
        return Err(format!("{path}: no [[port]] entries — nothing to bridge"));
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_fleet_config() {
        let text = r#"
            hub = "ws://server:9000"
            node = "pi-1"
            [[port]]
            path = "/dev/ttyACM0"
            baud = 115200
            alias = "uno"
            framing = "line"
            [[port]]
            path = "/dev/ttyUSB0"
        "#;
        let cfg: AgentConfig = toml::from_str(text).unwrap();
        assert_eq!(cfg.node, "pi-1");
        assert_eq!(cfg.ports.len(), 2);
        assert_eq!(cfg.ports[0].alias(), "uno");
        assert_eq!(cfg.ports[0].framing(), Framing::Line);
        // Defaults: alias from path basename, baud 9600, raw framing.
        assert_eq!(cfg.ports[1].alias(), "ttyUSB0");
        assert_eq!(cfg.ports[1].baud, 9600);
        assert_eq!(cfg.ports[1].framing(), Framing::Raw);
    }
}
