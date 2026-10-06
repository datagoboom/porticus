// Default per-device colors. A device with no assigned color gets a stable one
// derived from its key, so the feed is readable before anyone picks colors.

export const PALETTE = [
  '#4ade80', // green
  '#60a5fa', // blue
  '#f5b942', // amber
  '#c084fc', // violet
  '#f472b6', // pink
  '#22d3ee', // cyan
  '#fb923c', // orange
  '#a3e635', // lime
]

export function defaultColor(key: string): string {
  let h = 0
  for (let i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) >>> 0
  return PALETTE[h % PALETTE.length]
}
