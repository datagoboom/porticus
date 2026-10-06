import { useEffect, useRef, useState } from 'preact/hooks'
import type { ChatEntry, ViewMode } from '../types'
import { formatTime, toAscii, toHex } from '../format'
import { ScrollDown } from '../icons'

export interface DeviceLabel {
  name: string
  color: string
}

interface Props {
  entries: ChatEntry[]
  label: (key: string) => DeviceLabel
  view: ViewMode
  showTime: boolean
  wrap: boolean
  paused: boolean
}

/** The merged multi-device feed. Each row is tagged with its device's name in
 *  that device's color — the "chat" look. */
export function ChatView({ entries, label, view, showTime, wrap, paused }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const [atBottom, setAtBottom] = useState(true)

  useEffect(() => {
    const el = ref.current
    if (!el || paused || !atBottom) return
    el.scrollTop = el.scrollHeight
  }, [entries, paused, atBottom])

  const onScroll = () => {
    const el = ref.current
    if (!el) return
    setAtBottom(el.scrollHeight - el.scrollTop - el.clientHeight < 24)
  }

  const jump = () => {
    const el = ref.current
    if (!el) return
    el.scrollTop = el.scrollHeight
    setAtBottom(true)
  }

  return (
    <div class="streamwrap">
      <div
        ref={ref}
        class={`stream chat${wrap ? ' wrap' : ''}`}
        onScroll={onScroll}
        role="log"
        aria-live="polite"
      >
        {entries.length === 0 && <div class="stream-empty">no messages yet</div>}
        {entries.map((e) => {
          const d = label(e.key)
          const body =
            e.dir === 'sys' ? e.text : view === 'hex' ? toHex(e.bytes) : toAscii(e.bytes)
          return (
            <div key={e.id} class={`row row-${e.dir}`}>
              {showTime && <span class="ts">{formatTime(e.ts)}</span>}
              <span class="chat-name" style={{ color: d.color }}>
                {e.dir === 'tx' ? `you→${d.name}` : d.name}
              </span>
              <span class="payload">{body}</span>
            </div>
          )
        })}
      </div>
      {!atBottom && (
        <button class="jump" onClick={jump} title="Jump to latest">
          <ScrollDown size={14} /> latest
        </button>
      )}
    </div>
  )
}
