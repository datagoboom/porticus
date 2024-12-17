# Porticus

Serial <-> WebSocket Bridge. Connects serial devices to WebSocket clients.

## Install

Download the [latest release](https://github.com/datagoboom/porticus/releases/latest) for your platform.

Or build from source:
```bash
cargo build --release
```

## Usage

Basic usage:
```bash 
porticus -p /dev/ttyACM0 -b 9600 -w 8080
```

All options:
```
-p, --port <PORT>                     Serial port path
-b, --baud <BAUD>                     Baud rate [default: 9600]
-w, --websocket-port <PORT>           WebSocket port [default: 8080]
    --websocket-host <HOST>           WebSocket host [default: 127.0.0.1] 
    --buffer-size <SIZE>              Serial buffer size [default: 1024]
    --broadcast-capacity <CAP>         Broadcast channel capacity [default: 16]
    --kill                            Kill running instance
    --debug                           Enable debug logging
-q, --quiet                           Silence all output
```

## Architecture

The program is split into modules:
- `config.rs` - Configuration structs and defaults
- `error.rs` - Error handling
- `serial.rs` - Serial port management 
- `websocket.rs` - WebSocket server
- `main.rs` - CLI and orchestration

## Why did I build this?

I needed a way to connect a serial device to a WebSocket server for future projects. I also wanted to learn Rust. I'm not a professional Rust developer, so I'm sure there are many ways to improve this code. I'm open to suggestions and PRs.

## Contributing

1. Fork repository
2. Create feature branch 
3. Make changes
4. Add tests if applicable
5. Submit PR

Bug reports and feature requests welcome in Issues.

## License

MIT, see LICENSE file.