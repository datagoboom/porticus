// Rendering helpers for raw serial bytes.

/** Two-digit zero-padded milliseconds timestamp, local time: HH:MM:SS.mmm */
export function formatTime(ts: number): string {
  const d = new Date(ts)
  const p = (n: number, w = 2) => String(n).padStart(w, '0')
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`
}

/** Printable ASCII as-is; everything else as a dim middle dot. Control chars
 *  that matter visually (tab) are kept. */
export function toAscii(bytes: Uint8Array): string {
  let s = ''
  for (const b of bytes) {
    if (b === 0x09) s += '\t'
    else if (b >= 0x20 && b < 0x7f) s += String.fromCharCode(b)
    else s += '·'
  }
  return s
}

/** Space-separated hex, uppercase, grouped in bytes. */
export function toHex(bytes: Uint8Array): string {
  let s = ''
  for (let i = 0; i < bytes.length; i++) {
    s += bytes[i].toString(16).toUpperCase().padStart(2, '0')
    if (i < bytes.length - 1) s += ' '
  }
  return s
}

/** Parse a user-typed hex string ("DE AD 00" / "dead00") into bytes, or null if
 *  it isn't valid hex. */
export function parseHex(input: string): Uint8Array | null {
  const clean = input.replace(/[\s,]+/g, '')
  if (clean.length === 0) return new Uint8Array(0)
  if (clean.length % 2 !== 0 || /[^0-9a-fA-F]/.test(clean)) return null
  const out = new Uint8Array(clean.length / 2)
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16)
  }
  return out
}

const encoder = new TextEncoder()
export function encodeUtf8(s: string): Uint8Array {
  return encoder.encode(s)
}
