import { useState } from 'preact/hooks'
import type { LineEnding } from '../types'
import { LINE_ENDING_BYTES } from '../types'
import { encodeUtf8, parseHex } from '../format'
import { Send } from '../icons'

interface Props {
  framing?: string
  connected: boolean
  onSend: (bytes: Uint8Array) => void
}

const ENDINGS: { value: LineEnding; label: string }[] = [
  { value: 'none', label: 'no end' },
  { value: 'lf', label: 'LF' },
  { value: 'crlf', label: 'CRLF' },
  { value: 'cr', label: 'CR' },
]

export function Composer({ framing, connected, onSend }: Props) {
  const [text, setText] = useState('')
  const [hexMode, setHexMode] = useState(false)
  // When the bridge frames the stream itself, it already appends a terminator,
  // so default to adding none here to avoid doubling it.
  const framed = !!framing && framing !== 'raw'
  const [ending, setEnding] = useState<LineEnding>(framed ? 'none' : 'crlf')
  const [invalid, setInvalid] = useState(false)

  const submit = (e: Event) => {
    e.preventDefault()
    if (!connected) return
    let payload: Uint8Array | null
    if (hexMode) {
      payload = parseHex(text)
      if (payload === null) {
        setInvalid(true)
        return
      }
    } else {
      const body = encodeUtf8(text)
      const tail = Uint8Array.from(LINE_ENDING_BYTES[ending])
      payload = new Uint8Array(body.length + tail.length)
      payload.set(body, 0)
      payload.set(tail, body.length)
    }
    onSend(payload)
    setText('')
    setInvalid(false)
  }

  return (
    <form class="composer" onSubmit={submit}>
      <div class="segmented small" role="group" aria-label="input mode">
        <button type="button" class={!hexMode ? 'seg active' : 'seg'} onClick={() => setHexMode(false)}>
          text
        </button>
        <button type="button" class={hexMode ? 'seg active' : 'seg'} onClick={() => setHexMode(true)}>
          hex
        </button>
      </div>

      <input
        class={`composer-input${invalid ? ' invalid' : ''}`}
        value={text}
        placeholder={hexMode ? 'DE AD BE EF' : connected ? 'type a command…' : 'disconnected'}
        disabled={!connected}
        spellcheck={false}
        autocomplete="off"
        onInput={(e) => {
          setText((e.target as HTMLInputElement).value)
          if (invalid) setInvalid(false)
        }}
      />

      {!hexMode && (
        <select
          class="ending"
          value={ending}
          title="Line ending appended to each message"
          onChange={(e) => setEnding((e.target as HTMLSelectElement).value as LineEnding)}
        >
          {ENDINGS.map((o) => (
            <option value={o.value}>{o.label}</option>
          ))}
        </select>
      )}

      <button class="send" type="submit" disabled={!connected} title="Send (Enter)">
        <Send size={14} />
      </button>
    </form>
  )
}
