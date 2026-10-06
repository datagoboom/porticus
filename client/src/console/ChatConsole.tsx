import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks'
import type { ChatEntry, ConnState, Direction, FleetDevice, FleetNode, ViewMode } from '../types'
import { deviceKey } from '../types'
import type { Connection } from '../connection'
import { WsConnection } from '../connection'
import { ChatView, type DeviceLabel } from '../components/ChatView'
import { DevicePanel } from '../components/DevicePanel'
import { Toolbar } from '../components/Toolbar'
import { ChatComposer } from '../components/ChatComposer'
import { computeHandles, type Handle } from '../chat/handles'
import { setDeviceMeta } from '../chat/api'
import { defaultColor } from '../chat/palette'
import { formatTime, toAscii, toHex } from '../format'

const MAX_ENTRIES = 5000

/** The fleet experience as a chat: one live connection per device, a merged
 *  feed, and a right-side roster. Click a device to focus (filter + send
 *  target). */
export function ChatConsole({ initial }: { initial: FleetNode[] }) {
  const [nodes, setNodes] = useState<FleetNode[]>(initial)
  const [entries, setEntries] = useState<ChatEntry[]>([])
  const [states, setStates] = useState<Record<string, ConnState>>({})
  const [filter, setFilter] = useState<string | null>(null)
  const [unread, setUnread] = useState<Record<string, number>>({})
  // Instant-feedback overrides for nick/color while the hub persists them.
  const [overrides, setOverrides] = useState<Record<string, { nick?: string; color?: string }>>({})
  const [view, setView] = useState<ViewMode>('ascii')
  const [showTime, setShowTime] = useState(true)
  const [wrap, setWrap] = useState(true)
  const [paused, setPaused] = useState(false)

  const conns = useRef<Map<string, Connection>>(new Map())
  const idRef = useRef(0)
  const pending = useRef<ChatEntry[]>([])
  const raf = useRef<number | undefined>(undefined)
  const filterRef = useRef(filter)
  filterRef.current = filter

  // Count device output that arrives while you're focused on a different device.
  const bumpUnread = useCallback((key: string) => {
    const f = filterRef.current
    if (f !== null && f !== key) setUnread((p) => ({ ...p, [key]: (p[key] ?? 0) + 1 }))
  }, [])

  // Focus a device (or 'all'); clear its unread badge.
  const focus = useCallback((key: string | null) => {
    setFilter(key)
    if (key === null) setUnread({})
    else
      setUnread((p) => {
        const q = { ...p }
        delete q[key]
        return q
      })
  }, [])

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
    (key: string, dir: Direction, bytes: Uint8Array) => {
      pending.current.push({ id: idRef.current++, ts: Date.now(), key, dir, bytes })
      if (raf.current === undefined) raf.current = requestAnimationFrame(flush)
    },
    [flush],
  )

  // Poll the registry; devices come and go.
  useEffect(() => {
    let alive = true
    const tick = async () => {
      try {
        const r = await fetch('/fleet')
        if (r.ok && alive) setNodes(((await r.json()).nodes as FleetNode[]) ?? [])
      } catch {
        /* keep last known */
      }
    }
    const id = window.setInterval(tick, 4000)
    return () => {
      alive = false
      clearInterval(id)
    }
  }, [])

  // Reconcile one websocket per device against the current fleet.
  useEffect(() => {
    const want = new Set<string>()
    const proto = location.protocol === 'https:' ? 'wss' : 'ws'
    for (const n of nodes) {
      for (const d of n.devices) {
        const key = deviceKey(n.node, d.alias)
        want.add(key)
        if (!conns.current.has(key)) {
          const url = `${proto}://${location.host}/${n.node}/${d.alias}`
          const c = new WsConnection(url, {
            onMessage: (bytes) => {
              addEntry(key, 'rx', bytes)
              bumpUnread(key)
            },
            onState: (s) => setStates((p) => ({ ...p, [key]: s })),
          })
          conns.current.set(key, c)
        }
      }
    }
    for (const key of [...conns.current.keys()]) {
      if (!want.has(key)) {
        conns.current.get(key)!.close()
        conns.current.delete(key)
        setStates((p) => {
          const q = { ...p }
          delete q[key]
          return q
        })
      }
    }
  }, [nodes, addEntry, bumpUnread])

  useEffect(() => {
    const map = conns.current
    return () => map.forEach((c) => c.close())
  }, [])

  // Lookup helpers.
  const deviceMap = useMemo(() => {
    const m = new Map<string, FleetDevice>()
    for (const n of nodes) for (const d of n.devices) m.set(deviceKey(n.node, d.alias), d)
    return m
  }, [nodes])

  // Device identities (unique handle, display name, color), with local overrides
  // layered on top of the hub's values for instant feedback.
  const handles = useMemo(() => {
    const base = computeHandles(nodes)
    for (const [key, o] of Object.entries(overrides)) {
      const h = base.get(key)
      if (!h) continue
      if (o.nick !== undefined) h.name = o.nick.trim() || h.alias
      if (o.color !== undefined && o.color.trim()) h.color = o.color
    }
    return base
  }, [nodes, overrides])

  const handle = useCallback(
    (key: string): Handle =>
      handles.get(key) ?? {
        key,
        node: key.split('/')[0] ?? '',
        alias: key.split('/').pop() ?? key,
        name: key.split('/').pop() ?? key,
        color: defaultColor(key),
        handle: key,
      },
    [handles],
  )

  const label = useCallback((key: string): DeviceLabel => {
    const h = handle(key)
    return { name: h.name, color: h.color }
  }, [handle])

  // Raw stored values (not the derived defaults) so one edit doesn't clobber the
  // other field when we POST both.
  const rawNick = useCallback(
    (key: string) => overrides[key]?.nick ?? deviceMap.get(key)?.nick ?? '',
    [overrides, deviceMap],
  )
  const rawColor = useCallback(
    (key: string) => overrides[key]?.color ?? deviceMap.get(key)?.color ?? '',
    [overrides, deviceMap],
  )

  const onRename = useCallback(
    (key: string, nick: string) => {
      const h = handle(key)
      setOverrides((p) => ({ ...p, [key]: { ...p[key], nick } }))
      void setDeviceMeta(h.node, h.alias, nick, rawColor(key))
    },
    [handle, rawColor],
  )

  const onColor = useCallback(
    (key: string, color: string) => {
      const h = handle(key)
      setOverrides((p) => ({ ...p, [key]: { ...p[key], color } }))
      void setDeviceMeta(h.node, h.alias, rawNick(key), color)
    },
    [handle, rawNick],
  )

  const state = useCallback((key: string): ConnState => states[key] ?? 'connecting', [states])

  const shown = useMemo(
    () => (filter ? entries.filter((e) => e.key === filter) : entries),
    [entries, filter],
  )

  const onSend = useCallback(
    (keys: string[], bytes: Uint8Array) => {
      for (const key of keys) {
        conns.current.get(key)?.send(bytes)
        addEntry(key, 'tx', bytes)
      }
    },
    [addEntry],
  )

  const onClear = useCallback(() => {
    pending.current = []
    setEntries([])
  }, [])

  const onDownload = useCallback(() => {
    const text = shown
      .map((e) => {
        const name = label(e.key).name
        const tag = e.dir === 'tx' ? `you>${name}` : name
        const body = view === 'hex' ? toHex(e.bytes) : toAscii(e.bytes)
        return `${formatTime(e.ts)} ${tag}\t${body}`
      })
      .join('\n')
    const blob = new Blob([text], { type: 'text/plain' })
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob)
    a.download = `porticus-chat-${new Date().toISOString().replace(/[:.]/g, '-')}.log`
    a.click()
    URL.revokeObjectURL(a.href)
  }, [shown, view, label])

  return (
    <div class="chatwrap">
      <div class="chat-main">
        <header class="statusbar">
          <div class="statusbar-left">
            <span class="brand">porticus</span>
            <span class="chat-scope">{filter ? label(filter).name : 'all devices'}</span>
          </div>
          <div class="statusbar-right">
            <span class="meta">{conns.current.size} connected</span>
          </div>
        </header>
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
          count={shown.length}
        />
        <ChatView
          entries={shown}
          label={label}
          view={view}
          showTime={showTime}
          wrap={wrap}
          paused={paused}
        />
        <ChatComposer
          handles={[...handles.values()]}
          focused={filter}
          online={(k) => state(k) === 'connected'}
          onSend={onSend}
        />
      </div>
      <DevicePanel
        nodes={nodes}
        state={state}
        filter={filter}
        onFilter={focus}
        handle={handle}
        unread={(k) => unread[k] ?? 0}
        onRename={onRename}
        onColor={onColor}
      />
    </div>
  )
}
