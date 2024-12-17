pub mod config;
pub mod error;
pub mod serial;
pub mod websocket;

pub const BANNER: &str = r#"
 _____           _   _                 
|  __ \         | | (_)                
| |__) |__  _ __| |_ _  ___ _   _ ___ 
|  ___/ _ \| '__| __| |/ __| | | / __|
| |  | (_) | |  | |_| | (__| |_| \__ \
|_|   \___/|_|   \__|_|\___|\__,_|___/

by @datagoboom

Serial <-> WebSocket Bridge
"#;
