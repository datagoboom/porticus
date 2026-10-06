import { useCallback, useEffect, useRef, useState } from 'preact/hooks'
import type { ConnState, Direction, LogEntry, ViewMode } from '../types'
import type { Connection, Handlers } from '../connection'
import { StatusBar } from './StatusBar'
import { Toolbar } from './Toolbar'
import { StreamView } from './StreamView'
import { Composer } from './Composer'
import { formatTime, toAscii, toHex } from '../format'

// Scrollback cap: old lines fall off the top so a chatty device can't grow the
// DOM (or memory) without bound.
const MAX_ENTRIES = 5000

interface Props {
  title: string
  chips?: string[]
  framing?: string
  /** Opens the connection for this device. Captured once on mount. */
  makeConnection: (h: Handlers) => Connection
}

/** A single device's console: status bar, toolbar, scrollback, and composer,
 *  over whatever connection `makeConnection` returns. Reused for the standalone
 *  bridge and for each device in the fleet dashboard. */
export function DeviceView({ title, chips, framing, makeConnection }: Props) {
  const [state, setState] = useState<ConnState>('connecting')
  const [entries, setEntries] = useState<LogEntry[]>([])
  const [view, setView] = useState<ViewMode>('ascii')
  const [showTime, setShowTime] = useState(true)
  const [wrap, setWrap] = useState(true)
  const [paused, setPaused] = useState(false)
  const [rxBytes, setRxBytes] = useState(0)
  const [txBytes, setTxBytes] = useState(0)

  const conn = useRef<Connection | null>(null)
  const idRef = useRef(0)
  const pending = useRef<LogEntry[]>([])
  const raf = useRef<number | undefined>(undefined)
  const mk = useRef(makeConnection) // capture once; remount (via key) to re-connect

  const flush = useCallback(() => {
    raf.current = undefined
    if (pending.current.length === 0) return
    const batch = pending.current
    pending.current = []
    setEntries((prev) => {
      const next = prev.concat(batch)
      return next.length > MAX_ENTRIES ? next.slice(next.length - MAX_ENTRIES) : next
    })
  }, [])

  const addEntry = useCallback(
    (dir: Direction, bytes: Uint8Array, text?: string) => {
      pending.current.push({ id: idRef.current++, ts: Date.now(), dir, bytes, text })
      if (raf.current === undefined) raf.current = requestAnimationFrame(flush)
    },
    [flush],
  )

  useEffect(() => {
    const handlers: Handlers = {
      onMessage: (bytes) => {
        setRxBytes((n) => n + bytes.length)
        addEntry('rx', bytes)
      },
      onState: (s) => setState(s),
    }
    const c = mk.current(handlers)
    conn.current = c
    return () => c.close()
  }, [addEntry])

  const onSend = useCallback(
    (bytes: Uint8Array) => {
      conn.current?.send(bytes)
      setTxBytes((n) => n + bytes.length)
      addEntry('tx', bytes)
    },
    [addEntry],
  )

  const onClear = useCallback(() => {
    pending.current = []
    setEntries([])
  }, [])

  const onDownload = useCallback(() => {
    const text = entries
      .map((e) => {
        const glyph = e.dir === 'rx' ? '<' : e.dir === 'tx' ? '>' : '*'
        const body =
          e.dir === 'sys' ? e.text ?? '' : view === 'hex' ? toHex(e.bytes) : toAscii(e.bytes)
        return `${formatTime(e.ts)} ${glyph} ${body}`
      })
      .join('\n')
    const blob = new Blob([text], { type: 'text/plain' })
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob)
    a.download = `porticus-${new Date().toISOString().replace(/[:.]/g, '-')}.log`
    a.click()
    URL.revokeObjectURL(a.href)
  }, [entries, view])

  return (
    <div class="app">
      <StatusBar title={title} state={state} chips={chips} rxBytes={rxBytes} txBytes={txBytes} />
      <Toolbar
        view={view}
        onView={setView}
        showTime={showTime}
        onShowTime={setShowTime}
        wrap={wrap}
        onWrap={setWrap}
        paused={paused}
        onPaused={setPaused}
        onClear={onClear}
        onDownload={onDownload}
        count={entries.length}
      />
      <StreamView entries={entries} view={view} showTime={showTime} wrap={wrap} paused={paused} />
      <Composer framing={framing} connected={state === 'connected'} onSend={onSend} />
    </div>
  )
}
