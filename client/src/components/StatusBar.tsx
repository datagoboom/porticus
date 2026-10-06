import type { ConnState } from '../types'

interface Props {
  title: string
  state: ConnState
  chips?: string[]
  rxBytes: number
  txBytes: number
}

const LABEL: Record<ConnState, string> = {
  connecting: 'connecting',
  connected: 'connected',
  disconnected: 'reconnecting',
}

export function StatusBar({ title, state, chips, rxBytes, txBytes }: Props) {
  return (
    <header class="statusbar">
      <div class="statusbar-left">
        <span class="brand">{title}</span>
        <span class={`conn conn-${state}`}>
          <span class="dot" />
          {LABEL[state]}
        </span>
      </div>
      <div class="statusbar-right">
        {chips?.map((c) => (
          <span class="meta" key={c}>
            {c}
          </span>
        ))}
        <span class="counter" title="bytes received / sent">
          <span class="rx">↓ {fmtBytes(rxBytes)}</span>
          <span class="tx">↑ {fmtBytes(txBytes)}</span>
        </span>
      </div>
    </header>
  )
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}
