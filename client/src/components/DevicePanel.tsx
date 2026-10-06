import { useState } from 'preact/hooks'
import type { ConnState, FleetNode } from '../types'
import { deviceKey } from '../types'
import type { Handle } from '../chat/handles'
import { PALETTE } from '../chat/palette'
import { Pencil } from '../icons'

interface Props {
  nodes: FleetNode[]
  state: (key: string) => ConnState
  filter: string | null
  onFilter: (key: string | null) => void
  handle: (key: string) => Handle
  unread: (key: string) => number
  onRename: (key: string, nick: string) => void
  onColor: (key: string, color: string) => void
}

/** Right-side roster: every device grouped by node, with inline rename and a
 *  color picker. Click a device to focus the feed; click its @handle hint to see
 *  how to mention it. */
export function DevicePanel({
  nodes,
  state,
  filter,
  onFilter,
  handle,
  unread,
  onRename,
  onColor,
}: Props) {
  const total = nodes.reduce((a, n) => a + n.devices.length, 0)
  const [editing, setEditing] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [picking, setPicking] = useState<string | null>(null)

  const beginEdit = (h: Handle) => {
    setPicking(null)
    setEditing(h.key)
    setDraft(h.name)
  }
  const commit = (key: string) => {
    onRename(key, draft.trim())
    setEditing(null)
  }

  return (
    <aside class="roster">
      <div class="roster-head">
        devices
        <span class="roster-count">{total}</span>
      </div>

      <button class={`roster-all${filter === null ? ' active' : ''}`} onClick={() => onFilter(null)}>
        all devices
      </button>

      {nodes.length === 0 && <div class="roster-empty">no nodes connected</div>}

      {nodes.map((n) => (
        <div class="roster-node" key={n.node}>
          <div class="roster-node-name">{n.node}</div>
          {n.devices.map((d) => {
            const key = deviceKey(n.node, d.alias)
            const h = handle(key)
            const st = state(key)
            const isEditing = editing === key
            return (
              <div key={key} class={`roster-dev${filter === key ? ' active' : ''}`}>
                <button
                  class={`roster-swatch state-${st}`}
                  title={`Set color · ${st}`}
                  style={{ background: h.color }}
                  onClick={() => setPicking(picking === key ? null : key)}
                />
                {isEditing ? (
                  <input
                    class="roster-edit"
                    value={draft}
                    autoFocus
                    spellcheck={false}
                    placeholder={d.alias}
                    onInput={(e) => setDraft((e.target as HTMLInputElement).value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') commit(key)
                      else if (e.key === 'Escape') setEditing(null)
                    }}
                    onBlur={() => commit(key)}
                  />
                ) : (
                  <button class="roster-pick" onClick={() => onFilter(key)}>
                    <span class="roster-dev-name" style={{ color: h.color }}>
                      {h.name}
                    </span>
                    <span class="roster-dev-handle" title="mention handle">
                      @{h.handle}
                    </span>
                  </button>
                )}
                {!isEditing && unread(key) > 0 && (
                  <span class="roster-unread">{unread(key)}</span>
                )}
                {!isEditing && (
                  <button class="roster-rename" title="Rename" onClick={() => beginEdit(h)}>
                    <Pencil size={12} />
                  </button>
                )}
                {picking === key && (
                  <div class="roster-palette">
                    {PALETTE.map((c) => (
                      <button
                        key={c}
                        class="roster-pal"
                        style={{ background: c }}
                        onClick={() => {
                          onColor(key, c)
                          setPicking(null)
                        }}
                      />
                    ))}
                  </div>
                )}
              </div>
            )
          })}
        </div>
      ))}
    </aside>
  )
}
