export type Direction = 'rx' | 'tx' | 'sys'

/** One entry in the scrollback. `bytes` is the raw payload; `text` is used only
 *  for synthetic system lines (connect/disconnect notices). */
export interface LogEntry {
  id: number
  ts: number // epoch milliseconds
  dir: Direction
  bytes: Uint8Array
  text?: string
}

export type ConnState = 'connecting' | 'connected' | 'disconnected'

/** Shape of GET /info from the bridge: everything the console needs to label
 *  itself and locate the websocket. */
export interface Info {
  serial_port: string
  baud_rate: number
  framing: string
  websocket_port: number
  websocket_host: string
}

/** A device as reported by the hub's GET /fleet. */
export interface FleetDevice {
  alias: string
  baud: number
  framing: string
  /** User-assigned nickname (hub-stored); null/absent = use the alias. */
  nick?: string | null
  /** User-assigned color (hub-stored); null/absent = a default is derived. */
  color?: string | null
}

export interface FleetNode {
  node: string
  devices: FleetDevice[]
}

/** One line in the merged multi-device chat feed. `key` is the device it belongs
 *  to (node/alias). */
export interface ChatEntry {
  id: number
  ts: number
  key: string
  dir: Direction
  bytes: Uint8Array
  text?: string
}

/** Stable identity for a device across the UI: "<node>/<alias>". */
export function deviceKey(node: string, alias: string): string {
  return `${node}/${alias}`
}

export type ViewMode = 'ascii' | 'hex'

export type LineEnding = 'none' | 'lf' | 'crlf' | 'cr'

export const LINE_ENDING_BYTES: Record<LineEnding, number[]> = {
  none: [],
  lf: [0x0a],
  crlf: [0x0d, 0x0a],
  cr: [0x0d],
}
