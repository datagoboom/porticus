import { useCallback, useEffect, useMemo, useState } from 'preact/hooks'
import type { FleetDevice, FleetNode } from '../types'
import type { Connection, Handlers } from '../connection'
import { WsConnection } from '../connection'
import { DeviceView } from '../components/DeviceView'

interface Selection {
  node: string
  device: FleetDevice
}

/** The hub dashboard: a sidebar of every node/device (polled from /fleet) and a
 *  live console for the selected device, reusing DeviceView. */
export function FleetConsole({ initial }: { initial: FleetNode[] }) {
  const [nodes, setNodes] = useState<FleetNode[]>(initial)
  const [sel, setSel] = useState<Selection | null>(firstDevice(initial))

  // Poll the registry so nodes/devices appear and disappear live.
  useEffect(() => {
    let alive = true
    const tick = async () => {
      try {
        const r = await fetch('/fleet')
        if (r.ok && alive) setNodes(((await r.json()).nodes as FleetNode[]) ?? [])
      } catch {
        /* keep last known fleet */
      }
    }
    const id = window.setInterval(tick, 4000)
    return () => {
      alive = false
      clearInterval(id)
    }
  }, [])

  // Auto-select the first device once some appear.
  useEffect(() => {
    if (!sel) setSel(firstDevice(nodes))
  }, [nodes, sel])

  const online = useMemo(() => {
    if (!sel) return false
    return nodes.some((n) => n.node === sel.node && n.devices.some((d) => d.alias === sel.device.alias))
  }, [nodes, sel])

  const makeConnection = useCallback(
    (h: Handlers): Connection => {
      const proto = location.protocol === 'https:' ? 'wss' : 'ws'
      return new WsConnection(`${proto}://${location.host}/${sel!.node}/${sel!.device.alias}`, h)
    },
    [sel],
  )

  return (
    <div class="fleet">
      <aside class="fleet-sidebar">
        <div class="fleet-head">
          fleet
          <span class="fleet-count">{countDevices(nodes)}</span>
        </div>
        {nodes.length === 0 && <div class="fleet-empty">no nodes connected</div>}
        {nodes.map((n) => (
          <div class="fleet-node" key={n.node}>
            <div class="fleet-node-name">{n.node}</div>
            {n.devices.map((d) => {
              const active = sel?.node === n.node && sel?.device.alias === d.alias
              return (
                <button
                  key={d.alias}
                  class={`fleet-dev${active ? ' active' : ''}`}
                  onClick={() => setSel({ node: n.node, device: d })}
                >
                  <span class="fleet-dev-dot" />
                  <span class="fleet-dev-name">{d.alias}</span>
                  <span class="fleet-dev-meta">
                    {d.baud} · {d.framing}
                  </span>
                </button>
              )
            })}
          </div>
        ))}
      </aside>

      <main class="fleet-main">
        {sel ? (
          <DeviceView
            key={`${sel.node}/${sel.device.alias}`}
            title={`${sel.node} / ${sel.device.alias}`}
            chips={online ? [`${sel.device.baud} baud`, sel.device.framing] : ['offline']}
            framing={sel.device.framing}
            makeConnection={makeConnection}
          />
        ) : (
          <div class="fleet-placeholder">select a device</div>
        )}
      </main>
    </div>
  )
}

function firstDevice(nodes: FleetNode[]): Selection | null {
  for (const n of nodes) {
    if (n.devices.length) return { node: n.node, device: n.devices[0] }
  }
  return null
}

function countDevices(nodes: FleetNode[]): string {
  const devices = nodes.reduce((a, n) => a + n.devices.length, 0)
  return `${nodes.length} node${nodes.length === 1 ? '' : 's'}, ${devices} dev`
}
