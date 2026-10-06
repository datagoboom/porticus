import type { FleetNode } from '../types'
import { deviceKey } from '../types'
import { defaultColor } from './palette'

export interface Handle {
  key: string
  node: string
  alias: string
  /** Display name: nickname if set, else alias. */
  name: string
  /** Color: user color if set, else a stable default. */
  color: string
  /** Auto, always-unique mention handle: the alias when unique across the whole
   *  fleet, else "<node>/<alias>". Exists for every device with no user action,
   *  so nothing is ever un-mentionable. */
  handle: string
}

/** Build the per-device identity map from the fleet. Guarantees a unique handle
 *  for every device (auto-qualifying colliding aliases with their node). */
export function computeHandles(nodes: FleetNode[]): Map<string, Handle> {
  // Which aliases appear on more than one device?
  const aliasCounts = new Map<string, number>()
  for (const n of nodes) {
    for (const d of n.devices) aliasCounts.set(d.alias, (aliasCounts.get(d.alias) ?? 0) + 1)
  }

  const out = new Map<string, Handle>()
  for (const n of nodes) {
    for (const d of n.devices) {
      const key = deviceKey(n.node, d.alias)
      const unique = (aliasCounts.get(d.alias) ?? 0) <= 1
      out.set(key, {
        key,
        node: n.node,
        alias: d.alias,
        name: d.nick && d.nick.trim() ? d.nick : d.alias,
        color: d.color && d.color.trim() ? d.color : defaultColor(key),
        handle: unique ? d.alias : `${n.node}/${d.alias}`,
      })
    }
  }
  return out
}
