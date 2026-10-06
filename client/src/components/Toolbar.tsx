import type { ViewMode } from '../types'
import { Pause, Play, Trash, Download } from '../icons'

interface Props {
  view: ViewMode
  onView: (v: ViewMode) => void
  showTime: boolean
  onShowTime: (b: boolean) => void
  wrap: boolean
  onWrap: (b: boolean) => void
  paused: boolean
  onPaused: (b: boolean) => void
  onClear: () => void
  onDownload: () => void
  count: number
}

export function Toolbar(p: Props) {
  return (
    <div class="toolbar">
      <div class="segmented" role="group" aria-label="view mode">
        <button class={p.view === 'ascii' ? 'seg active' : 'seg'} onClick={() => p.onView('ascii')}>
          ASCII
        </button>
        <button class={p.view === 'hex' ? 'seg active' : 'seg'} onClick={() => p.onView('hex')}>
          HEX
        </button>
      </div>

      <button
        class={p.showTime ? 'toggle active' : 'toggle'}
        onClick={() => p.onShowTime(!p.showTime)}
        title="Toggle timestamps"
      >
        time
      </button>
      <button
        class={p.wrap ? 'toggle active' : 'toggle'}
        onClick={() => p.onWrap(!p.wrap)}
        title="Toggle line wrapping"
      >
        wrap
      </button>

      <div class="toolbar-spacer" />

      <span class="count">{p.count.toLocaleString()} lines</span>

      <button
        class={p.paused ? 'toggle active' : 'toggle'}
        onClick={() => p.onPaused(!p.paused)}
        title={p.paused ? 'Resume autoscroll' : 'Pause autoscroll'}
      >
        {p.paused ? <Play size={13} /> : <Pause size={13} />}
        {p.paused ? 'paused' : 'live'}
      </button>
      <button class="toggle" onClick={p.onDownload} title="Download log">
        <Download size={13} />
      </button>
      <button class="toggle danger" onClick={p.onClear} title="Clear">
        <Trash size={13} />
      </button>
    </div>
  )
}
