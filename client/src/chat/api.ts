// Hub device-metadata API (nicknames + colors). Persisted on the hub so every
// browser sees the same names.

export async function setDeviceMeta(
  node: string,
  alias: string,
  nick: string,
  color: string,
): Promise<boolean> {
  try {
    const r = await fetch('/meta', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ node, alias, nick, color }),
    })
    return r.ok
  } catch {
    return false
  }
}
