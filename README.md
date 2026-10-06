# Porticus

Serial <-> WebSocket Bridge. Connects serial devices to WebSocket clients.

Some notes:
- Serial data is delivered to clients as binary WebSocket messages, one message per serial read (not per byte)
- Multiple clients can connect at once; they all receive the same serial data, and all of their writes go to the device
- If the serial device disconnects, Porticus keeps retrying with backoff so clients can stay connected across a replug
- Process PID is stored in ~/.config/porticus/porticus.pid on *nix systems and cleaned up on exit

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
-p, --port <PORT>                     Serial port path [default: /dev/ttyACM0, COM1 on Windows]
-b, --baud <BAUD>                     Baud rate [default: 9600]
-w, --websocket-port <PORT>           WebSocket port [default: 8080]
    --websocket-host <HOST>           WebSocket host [default: 127.0.0.1]
    --buffer-size <BYTES>             Serial read buffer size [default: 1024]
    --broadcast-capacity <CAP>        Messages buffered per client [default: 16]
    --write-capacity <CAP>            Client-to-device messages buffered [default: 32]
    --framing <MODE>                  Message framing: raw|line|cobs|slip [default: raw]
    --http-port <PORT>                Browser console port, 0 to disable [default: 8081]
    --record <FILE>                   Record the session (both directions) to a capture file
    --replay <FILE>                   Replay a capture as a virtual device (no serial port opened)
    --script <FILE>                   Run a send/expect script, then exit with its result
    --kill                            Kill running instance
    --debug                           Enable debug logging
-q, --quiet                           Silence all output
```

## Browser console

Porticus serves a self-contained web serial monitor on its own HTTP port
(`http://<host>:8081/` by default) alongside the WebSocket bridge. Open it in a
browser to watch the stream live, toggle ASCII/HEX, see timestamps, send
commands (with configurable line endings or raw hex), and download a log. It
reconnects on its own if the bridge restarts.

The console is a Preact app built to a single inlined HTML file and embedded in
the binary — no extra files to ship. The WebSocket endpoint is unchanged, so
existing clients are unaffected. Disable the console with `--http-port 0`.

The same app powers the hub's **fleet dashboard**: it detects its context at load
(`/fleet` → multi-device dashboard, `/info` → single bridge) so one embedded page
serves both.

Rebuilding the UI (only needed when changing `client/` source):

```bash
cd client && npm install && npm run build   # writes client/dist/index.html
cargo build                                 # re-embeds it
```

## Framing

Serial is a byte stream with no message boundaries, so by default (`--framing raw`)
each WebSocket message is simply whatever one serial read returned. Set `--framing`
to split the stream into logical messages instead:

- `raw` — passthrough; one message per serial read (default).
- `line` — newline-delimited text. Emits one message per line (trailing `\r` trimmed);
  outgoing messages get a `\n` appended.
- `cobs` — COBS frames delimited by `0x00`. Symmetric; payload may contain any bytes.
- `slip` — SLIP frames (RFC 1055), delimited by `0xC0`. Symmetric.

Framing applies in both directions and buffers partial frames across reads, so a
message split across two serial reads still arrives as one WebSocket message.

## Record, replay & scripting

Capture a live session (both directions) to a simple, greppable file:

```bash
porticus -p /dev/ttyACM0 --record session.cap
```

Each line is `<elapsed_ms> <dir> <hex>` (`dir` is `rx` = device→clients or
`tx` = clients→device). Replay it later as a virtual device — no hardware
needed — so clients see the recorded stream at its original timing:

```bash
porticus --replay session.cap        # no serial port is opened
```

Run a send/expect script against a device and exit nonzero on the first failed
expectation (CI-friendly):

```bash
porticus -p /dev/ttyACM0 --script check.txt
```

```text
# check.txt
send AT\r\n
expect 1000 OK
send AT+VERSION\r\n
expect v1.
```

Script commands: `send <text>` (escapes `\n \r \t \0 \\ \xHH`), `send-hex <hex>`,
`expect [<ms>] <substr>`, `delay <ms>`. Script mode is headless — it does not
start the WebSocket server or console.

## Fleet mode

For many devices across several machines, run porticus as a **hub** and one or
more **agents**. Each agent (e.g. on a Raspberry Pi) exposes an allowlist of
serial ports and dials *out* to the hub (NAT-friendly — no inbound config on the
edge), multiplexing all its devices over a single connection. The hub aggregates
every device and re-exposes it to clients.

Hub:

```bash
porticus hub run --listen 0.0.0.0:9000 --http-port 8080 --tls
#   agents dial    wss://<hub>:9000                    (mutual TLS)
#   clients open   http://<hub>:8080/                   (unified console)
#                  GET http://<hub>:8080/fleet          (registry JSON)
#                  ws  ws://<hub>:8080/<node>/<device>   (live stream)
```

Open `http://<hub>:8080/` in a browser for the **fleet dashboard** — a sidebar of
every node and device, click one to stream it live (the same console UI as the
standalone bridge, reused per device).

Agent (`agent.toml`):

```toml
hub  = "wss://hub-host:9000"
node = "pi-bench-1"

[tls]                       # omit for a plaintext ws link (dev only)
ca   = "ca.crt"
cert = "pi-bench-1.crt"
key  = "pi-bench-1.key"

[[port]]
path    = "/dev/ttyACM0"
baud    = 115200
alias   = "uno"
framing = "line"

[[port]]
path = "/dev/ttyUSB0"
baud = 9600          # alias defaults to "ttyUSB0", framing to raw
```

```bash
porticus agent --config agent.toml
```

An agent only ever bridges ports named in its config. Both the agent↔hub link
and each serial port reconnect on their own with backoff. Standalone mode (no
subcommand) is unchanged.

### Mutual TLS

The hub is a small private CA. Set it up once, then enroll each agent:

```bash
porticus hub init --san hub.local --san 192.168.1.10   # addresses agents dial
porticus hub enroll pi-bench-1 --out ./pi-bench-1-certs # writes crt/key/ca.crt
```

Copy the generated `pi-bench-1.crt`, `pi-bench-1.key`, and `ca.crt` to the agent
and reference them in its `[tls]` block. The hub only accepts agents whose client
certificate chains to the CA **and** whose fingerprint is listed in
`~/.config/porticus/hub/enrolled.txt`; delete a line there to revoke that agent.
Run the hub with `--tls` to require this (without it, agents connect over plain
`ws` — dev only).

## Architecture

The program is split into modules:
- `config.rs` - Configuration structs and defaults
- `error.rs` - Error handling
- `framing.rs` - Message framing (raw/line/cobs/slip)
- `capture.rs` - Session record/replay format and tasks
- `script.rs` - Send/expect script runner
- `fleet/` - Fleet mode: `agent`, `hub`, mux `protocol`, agent `config` (TOML), `tls` (mTLS CA/enroll)
- `serial.rs` - Serial port management
- `websocket.rs` - WebSocket server
- `http.rs` - Browser console HTTP server (serves the embedded UI + `/info`)
- `main.rs` - CLI and orchestration

The browser console lives in `client/` (Preact + Vite, built to a single inlined
HTML file that `http.rs` embeds).

The serial task owns the port: incoming bytes fan out to clients over a broadcast channel, and client writes funnel back through an mpsc channel. Each WebSocket client runs in its own task.

## Tests

```bash
cargo test
```

Integration tests run the real bridge against a pseudo-terminal pair (unix only), so no hardware is needed.

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
