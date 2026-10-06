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
- framing: run with `--framing line`; a device burst of `first\r\nsecond\nthird\n` → clients get three separate messages with terminators stripped (not one 20-byte chunk)
- console: run with `--http-port 8899`; `curl http://127.0.0.1:8899/info` → JSON with the live ws port/baud/framing, `curl http://127.0.0.1:8899/` → the console HTML. For the full UI, open it with Playwright (dark via emulate_media), write serial lines from the device end, screenshot — rows should appear live. Built from `client/` (`npm run build` → `client/dist/index.html`, embedded via include_str!). NOTE: default `--http-port` is 8081, which may collide with mitmproxy on this machine — use a free port like 8899 when testing.
- record/replay: `--record cap.txt` then write device lines + a client send → file has `<ms> rx <hex>` / `tx <hex>` lines. `--replay cap.txt` (no serial port) → a ws client receives the recorded rx messages at original timing ("replaying N events" in the log).
- script: `--script ok.txt` with `send PING\r\n` / `expect 2000 PONG` against a python responder on ptyB that replies PONG to PING → exit 0; a never-matched `expect` → exit 1 with "script failed: ... timeout". Script mode is headless (no ws/console).
- fleet (hub+agent): start `porticus hub run --listen 127.0.0.1:9000 --http-host 127.0.0.1 --http-port 8090`, write an agent TOML (hub url, node, `[[port]]` entries pointing at socat ptyAs), start `porticus agent --config agent.toml`. Check `curl http://127.0.0.1:8090/fleet` lists the node+devices; `ws://127.0.0.1:8090/<node>/<alias>` streams a device (write to its ptyB → client sees it; client send → ptyB sees it). Framing is per-port in the agent config. Kill the hub → agent logs backoff reconnect; restart hub → re-registers and data flows. (NOTE: hub run is `hub run`, not `hub`. `pkill -f "porticus hub"` tends to exit 144 and abort a compound command — kill by PID instead.)
- fleet console (P3): the hub serves the unified dashboard at `http://<hub-http>/` (same embedded Preact app as the single-device console; it probes `/fleet` vs `/info` to pick mode). Open it with Playwright against a running hub+agent: sidebar lists nodes/devices, auto-selects the first, streams live; click another device to switch; typing in the composer writes to that device (browser→hub→agent→serial, verified by a reader on the device's ptyB). Rebuild the client (`cd client && npm run build`) after UI changes — the hub embeds `client/dist/index.html` via `crate::http::CONSOLE_HTML`.
- fleet mTLS: `porticus hub init --san 127.0.0.1 --san localhost` (writes CA+server cert to ~/.config/porticus/hub), `porticus hub enroll <node> --out <dir>` (bundle + fingerprint in enrolled.txt). Run hub with `--tls`, agent TOML gets a `[tls]` block (ca/cert/key) + `wss://` hub url → connects over mutual TLS. Revoke by removing the fingerprint line from ~/.config/porticus/hub/enrolled.txt → hub logs "rejected: certificate ... is not enrolled" even though the cert still chains to the CA. Clean up the test CA with `rm -rf ~/.config/porticus/hub` afterward.

## Gotchas

- Restart socat with `nohup ... & disown` — a plain background job in the same compound command as a `pkill socat` tends to die with it.
- PID file lives at `~/.config/porticus/porticus.pid`; remove it when done so later runs aren't confused.
- Log lines contain ANSI color; strip with `sed 's/\x1b\[[0-9;]*m//g'` when asserting on them.
