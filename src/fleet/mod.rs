//! Fleet mode: many agents, each exposing an allowlist of serial ports, dial out
//! to a central hub that aggregates every device and re-exposes it to clients.
//!
//! - [`agent`] runs on the edge (e.g. a Raspberry Pi): opens its ports and
//!   multiplexes them over one outbound connection to the hub.
//! - [`hub`] runs centrally: registers agents, keeps a `node → devices` registry,
//!   and serves `ws://hub/<node>/<device>` plus `GET /fleet` to clients.
//! - [`protocol`] is the agent↔hub multiplexing wire format.
//! - [`config`] is the agent's TOML configuration.

pub mod agent;
pub mod config;
pub mod hub;
pub mod protocol;
pub mod tls;
