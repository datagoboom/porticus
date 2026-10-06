import { useEffect, useRef, useState } from 'preact/hooks'
import type { LogEntry, ViewMode } from '../types'
import { formatTime, toAscii, toHex } from '../format'
import { ScrollDown } from '../icons'

interface Props {
  entries: LogEntry[]
  view: ViewMode
  showTime: boolean
  wrap: boolean
  paused: boolean
}

const DIR_GLYPH = { rx: '<', tx: '>', sys: '*' } as const

export function StreamView({ entries, view, showTime, wrap, paused }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const [atBottom, setAtBottom] = useState(true)

  // Follow the tail unless the user has scrolled up or hit pause.
  useEffect(() => {
    const el = ref.current
    if (!el || paused || !atBottom) return
    el.scrollTop = el.scrollHeight
  }, [entries, paused, atBottom])

  const onScroll = () => {
    const el = ref.current
    if (!el) return
    const dist = el.scrollHeight - el.scrollTop - el.clientHeight
    setAtBottom(dist < 24)
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
        class={`stream${wrap ? ' wrap' : ''}`}
        onScroll={onScroll}
        role="log"
        aria-live="polite"
      >
        {entries.length === 0 && (
          <div class="stream-empty">waiting for data…</div>
        )}
        {entries.map((e) => (
          <div key={e.id} class={`row row-${e.dir}`}>
            {showTime && <span class="ts">{formatTime(e.ts)}</span>}
            <span class="glyph">{DIR_GLYPH[e.dir]}</span>
            <span class="payload">
              {e.dir === 'sys' ? e.text : view === 'hex' ? toHex(e.bytes) : toAscii(e.bytes)}
            </span>
          </div>
        ))}
      </div>
      {!atBottom && (
        <button class="jump" onClick={jump} title="Jump to latest">
          <ScrollDown size={14} /> latest
        </button>
      )}
    </div>
  )
}
