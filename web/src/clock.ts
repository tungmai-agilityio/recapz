/** m:ss, or h:mm:ss past an hour. */
export function clock(ms: number) {
  const t = Math.floor(ms / 1000)
  const [h, m, s] = [Math.floor(t / 3600), Math.floor((t % 3600) / 60), t % 60]
  const mm = h ? String(m).padStart(2, '0') : String(m)
  return `${h ? `${h}:` : ''}${mm}:${String(s).padStart(2, '0')}`
}
