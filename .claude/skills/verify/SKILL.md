---
name: verify
description: Build and runtime-verify porticus (serial<->WebSocket bridge) end-to-end without hardware, using a socat PTY pair and a python websockets client.
---

# Verifying porticus

Build: `cargo build` (binary at `target/debug/porticus`).

## Fake serial device

```bash
socat -d -d pty,raw,echo=0,link=$SCRATCH/ptyA pty,raw,echo=0,link=$SCRATCH/ptyB &
```

Run porticus on one end: `./target/debug/porticus -p $SCRATCH/ptyA -b 115200 -w 8192`.
The other end (`ptyB`) acts as the device: `os.open(realpath, O_RDWR|O_NOCTTY)` from python, then `os.read`/`os.write`.

## Drive it

Python with the `websockets` package (installed system-wide) connecting to `ws://127.0.0.1:8192`. Flows worth driving:

- device write → all connected clients receive one binary message
- client binary/text send → bytes arrive at the device
- `curl http://127.0.0.1:8192/` → handshake fails but server keeps accepting
- kill socat → porticus logs open retries with backoff; restart socat → reconnects and data flows again (wait ~10s, backoff may be at 8-16s)
- `porticus --kill` → logs "shutting down", removes `~/.config/porticus/porticus.pid`
- stale PID: `echo 999999 > ~/.config/porticus/porticus.pid; porticus --kill` → reports stale, removes file

## Gotchas

- Restart socat with `nohup ... & disown` — a plain background job in the same compound command as a `pkill socat` tends to die with it.
- PID file lives at `~/.config/porticus/porticus.pid`; remove it when done so later runs aren't confused.
- Log lines contain ANSI color; strip with `sed 's/\x1b\[[0-9;]*m//g'` when asserting on them.
