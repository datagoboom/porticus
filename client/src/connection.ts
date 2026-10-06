import type { ConnState } from './types'

export interface Connection {
  send(bytes: Uint8Array): void
  close(): void
}

export interface Handlers {
  onMessage(bytes: Uint8Array): void
  onState(state: ConnState): void
}

/** A WebSocket connection to the bridge that transparently reconnects with
 *  exponential backoff — mirroring porticus's own serial reconnect, so the UI
 *  stays live across a bridge restart or a device replug. */
export class WsConnection implements Connection {
  private ws: WebSocket | null = null
  private closed = false
  private backoff = 500
  private readonly maxBackoff = 8000
  private timer: number | undefined

  constructor(
    private readonly url: string,
    private readonly h: Handlers,
  ) {
    this.connect()
  }

  private connect() {
    if (this.closed) return
    this.h.onState('connecting')
    const ws = new WebSocket(this.url)
    ws.binaryType = 'arraybuffer'
    this.ws = ws

    ws.onopen = () => {
      this.backoff = 500
      this.h.onState('connected')
    }
    ws.onmessage = (ev) => {
      if (ev.data instanceof ArrayBuffer) {
        this.h.onMessage(new Uint8Array(ev.data))
      } else if (typeof ev.data === 'string') {
        this.h.onMessage(new TextEncoder().encode(ev.data))
      }
    }
    ws.onclose = () => this.scheduleReconnect()
    ws.onerror = () => ws.close()
  }

  private scheduleReconnect() {
    if (this.closed) return
    this.h.onState('disconnected')
    this.timer = window.setTimeout(() => this.connect(), this.backoff)
    this.backoff = Math.min(this.backoff * 2, this.maxBackoff)
  }

  send(bytes: Uint8Array) {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(bytes)
    }
  }

  close() {
    this.closed = true
    if (this.timer) clearTimeout(this.timer)
    this.ws?.close()
  }
}

/** A fake device for design preview / offline dev: emits plausible serial
 *  chatter on a timer and echoes whatever you send. Used when /info can't be
 *  reached (e.g. the standalone design mockup). */
export class MockConnection implements Connection {
  private interval: number | undefined
  private lineTimers: number[] = []

  constructor(private readonly h: Handlers) {
    this.h.onState('connecting')
    window.setTimeout(() => this.h.onState('connected'), 300)
    const enc = new TextEncoder()
    let temp = 24.0
    let n = 0
    this.interval = window.setInterval(() => {
      n++
      temp += (Math.sin(n / 3) * 0.4)
      const lines = [
        `boot: stage ${n % 4} ok`,
        `temp=${temp.toFixed(2)}C rh=${(40 + (n % 7)).toFixed(0)}%`,
        `adc[0]=${(512 + ((n * 37) % 300)).toString()} adc[1]=${((n * 91) % 1024).toString()}`,
        `heartbeat seq=${n}`,
      ]
      this.h.onMessage(enc.encode(lines[n % lines.length]))
    }, 900)
  }

  send(bytes: Uint8Array) {
    // Echo the command back as a mock device acknowledgement.
    const enc = new TextEncoder()
    const txt = new TextDecoder().decode(bytes).trim()
    const t = window.setTimeout(() => {
      this.h.onMessage(enc.encode(`ack: ${txt || '(empty)'} -> OK`))
    }, 180)
    this.lineTimers.push(t)
  }

  close() {
    if (this.interval) clearInterval(this.interval)
    this.lineTimers.forEach(clearTimeout)
  }
}
