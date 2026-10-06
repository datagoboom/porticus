// Minimal inline icons (lucide-derived paths) so the console carries no icon
// font or external asset. Stroke-based, inherit currentColor.
import type { JSX } from 'preact'

type P = { size?: number } & JSX.SVGAttributes<SVGSVGElement>

function Svg({ size = 15, children, ...rest }: P & { children: preact.ComponentChildren }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      {...rest}
    >
      {children}
    </svg>
  )
}

export const Pause = (p: P) => (
  <Svg {...p}>
    <rect x="6" y="4" width="4" height="16" rx="1" />
    <rect x="14" y="4" width="4" height="16" rx="1" />
  </Svg>
)

export const Play = (p: P) => (
  <Svg {...p}>
    <polygon points="6 3 20 12 6 21 6 3" />
  </Svg>
)

export const Trash = (p: P) => (
  <Svg {...p}>
    <path d="M3 6h18" />
    <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
  </Svg>
)

export const Download = (p: P) => (
  <Svg {...p}>
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    <polyline points="7 10 12 15 17 10" />
    <line x1="12" y1="15" x2="12" y2="3" />
  </Svg>
)

export const Send = (p: P) => (
  <Svg {...p}>
    <path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z" />
    <path d="m21.854 2.147-10.94 10.939" />
  </Svg>
)

export const ScrollDown = (p: P) => (
  <Svg {...p}>
    <path d="M12 5v14" />
    <path d="m19 12-7 7-7-7" />
  </Svg>
)
