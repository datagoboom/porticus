import { useCallback } from 'preact/hooks'
import type { Info } from '../types'
import type { Connection, Handlers } from '../connection'
import { MockConnection, WsConnection } from '../connection'
import { DeviceView } from '../components/DeviceView'

/** Standalone single-port bridge console. With `info` it connects to the live
 *  websocket; otherwise it runs against a simulated device (design preview). */
export function SingleConsole({ info }: { info: Info | null }) {
  const makeConnection = useCallback(
    (h: Handlers): Connection => {
      if (!info) return new MockConnection(h)
      const proto = location.protocol === 'https:' ? 'wss' : 'ws'
      const host =
        info.websocket_host === '0.0.0.0' || info.websocket_host === '127.0.0.1'
          ? location.hostname
          : info.websocket_host
      return new WsConnection(`${proto}://${host}:${info.websocket_port}/`, h)
    },
    [info],
  )

  const chips = info
    ? [info.serial_port, `${info.baud_rate} baud`, info.framing]
    : ['preview — simulated device']

  return (
    <DeviceView
      title="porticus"
      chips={chips}
      framing={info?.framing}
      makeConnection={makeConnection}
    />
  )
}
