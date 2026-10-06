import { useMemo, useState } from 'preact/hooks'
import type { LineEnding } from '../types'
import { LINE_ENDING_BYTES } from '../types'
import type { Handle } from '../chat/handles'
import { encodeUtf8, parseHex } from '../format'
import { Send } from '../icons'

interface Props {
  handles: Handle[]
  focused: string | null // deviceKey of the focused device, or null (all view)
  online: (key: string) => boolean
  onSend: (keys: string[], bytes: Uint8Array) => void
}

const ENDINGS: { value: LineEnding; label: string }[] = [
  { value: 'none', label: 'no end' },
  { value: 'lf', label: 'LF' },
  { value: 'crlf', label: 'CRLF' },
  { value: 'cr', label: 'CR' },
]

interface Resolved {
  keys: string[]
  payload: string
  label: string
  error?: string
}

export function ChatComposer({ handles, focused, online, onSend }: Props) {
  const [text, setText] = useState('')
  const [hexMode, setHexMode] = useState(false)
  const [ending, setEnding] = useState<LineEnding>('crlf')
  const [invalid, setInvalid] = useState(false)

  // Resolve the current input to a target (an @mention, @all, or the focused
  // device) plus the payload to send.
  const resolved = useMemo<Resolved>(() => resolve(text, handles, focused, online), [
    text,
    handles,
    focused,
    online,
  ])

  // Autocomplete while typing a mention (`@partial` with no space yet).
  const suggestions = useMemo(() => {
    const m = text.match(/^@([^\s]*)$/)
    if (!m) return []
    const p = m[1].toLowerCase()
    const names = ['all', ...handles.map((h) => h.handle)]
    return names.filter((n) => n.toLowerCase().startsWith(p) && n.toLowerCase() !== p).slice(0, 6)
  }, [text, handles])

  const complete = (name: string) => setText(`@${name} `)

  const submit = (e: Event) => {
    e.preventDefault()
    if (resolved.error || resolved.keys.length === 0) return
    let payload: Uint8Array | null
    if (hexMode) {
      payload = parseHex(resolved.payload)
      if (payload === null) {
        setInvalid(true)
        return
      }
    } else {
      const body = encodeUtf8(resolved.payload)
      const tail = Uint8Array.from(LINE_ENDING_BYTES[ending])
      payload = new Uint8Array(body.length + tail.length)
      payload.set(body, 0)
      payload.set(tail, body.length)
    }
    onSend(resolved.keys, payload)
    setText('')
    setInvalid(false)
  }

  const canSend = !resolved.error && resolved.keys.length > 0

  return (
    <form class="composer chat-composer" onSubmit={submit}>
      {suggestions.length > 0 && (
        <div class="mention-pop">
          {suggestions.map((s) => (
            <button type="button" key={s} class="mention-opt" onMouseDown={(e) => e.preventDefault()} onClick={() => complete(s)}>
              @{s}
              {s === 'all' && <span class="mention-hint">broadcast</span>}
            </button>
          ))}
        </div>
      )}

      <span class={`chat-target${resolved.error ? ' err' : ''}`} title="where this goes">
        {resolved.error ?? `→ ${resolved.label}`}
      </span>

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
        placeholder={hexMode ? '@uno DE AD BE EF' : 'message, or @device …'}
        spellcheck={false}
        autocomplete="off"
        onInput={(e) => {
          setText((e.target as HTMLInputElement).value)
          if (invalid) setInvalid(false)
        }}
        onKeyDown={(e) => {
          if (e.key === 'Tab' && suggestions.length) {
            e.preventDefault()
            complete(suggestions[0])
          }
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

      <button class="send" type="submit" disabled={!canSend} title="Send (Enter)">
        <Send size={14} />
      </button>
    </form>
  )
}

function resolve(
  text: string,
  handles: Handle[],
  focused: string | null,
  online: (key: string) => boolean,
): Resolved {
  if (text.startsWith('@')) {
    const sp = text.indexOf(' ')
    const token = (sp === -1 ? text.slice(1) : text.slice(1, sp)).trim()
    const payload = sp === -1 ? '' : text.slice(sp + 1)
    if (token.toLowerCase() === 'all') {
      const keys = handles.filter((h) => online(h.key)).map((h) => h.key)
      return keys.length
        ? { keys, payload, label: `all (${keys.length})` }
        : { keys: [], payload, label: 'all', error: 'no devices online' }
    }
    const match = matchHandle(token, handles)
    if (!match) return { keys: [], payload, label: token, error: `unknown device @${token}` }
    return { keys: [match.key], payload, label: match.name }
  }

  // No mention: go to the focused device if there is one.
  if (focused) {
    const f = handles.find((h) => h.key === focused)
    return { keys: [focused], payload: text, label: f?.name ?? focused }
  }
  return { keys: [], payload: text, label: '', error: 'pick a device, or @mention one' }
}

function matchHandle(token: string, handles: Handle[]): Handle | undefined {
  const t = token.toLowerCase()
  return (
    handles.find((h) => h.handle.toLowerCase() === t) ??
    handles.find((h) => h.alias.toLowerCase() === t) ??
    handles.find((h) => h.name.toLowerCase().replace(/\s+/g, '') === t)
  )
}
