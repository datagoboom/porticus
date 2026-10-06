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
}

export interface FleetNode {
  node: string
  devices: FleetDevice[]
}

export type ViewMode = 'ascii' | 'hex'

export type LineEnding = 'none' | 'lf' | 'crlf' | 'cr'

export const LINE_ENDING_BYTES: Record<LineEnding, number[]> = {
  none: [],
  lf: [0x0a],
  crlf: [0x0d, 0x0a],
  cr: [0x0d],
}
