import { useEffect, useState } from 'preact/hooks'
import type { FleetNode, Info } from './types'
import { SingleConsole } from './console/SingleConsole'
import { FleetConsole } from './console/FleetConsole'

type Mode =
  | { kind: 'loading' }
  | { kind: 'fleet'; nodes: FleetNode[] }
  | { kind: 'single'; info: Info | null }

/** Auto-detects context: a hub (GET /fleet) → fleet dashboard, a standalone
 *  bridge (GET /info) → single console, neither → design preview. */
export function App() {
  const [mode, setMode] = useState<Mode>({ kind: 'loading' })

  useEffect(() => {
    let alive = true
    ;(async () => {
      try {
        const r = await fetch('/fleet')
        if (r.ok) {
          const nodes = ((await r.json()).nodes as FleetNode[]) ?? []
          if (alive) setMode({ kind: 'fleet', nodes })
          return
        }
      } catch {
        /* not a hub */
      }
      try {
        const r = await fetch('/info')
        if (r.ok) {
          const info = (await r.json()) as Info
          if (alive) setMode({ kind: 'single', info })
          return
        }
      } catch {
        /* not a bridge */
      }
      if (alive) setMode({ kind: 'single', info: null }) // preview
    })()
    return () => {
      alive = false
    }
  }, [])

  if (mode.kind === 'loading') return <div class="booting">connecting…</div>
  if (mode.kind === 'fleet') return <FleetConsole initial={mode.nodes} />
  return <SingleConsole info={mode.info} />
}
