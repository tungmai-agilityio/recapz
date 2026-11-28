/** A finished live line. `speaker` is the provider's 0-based number (one colour per number), `label` the name or "Speaker N" to show, `ms` how long it took to say. */
export interface Said { at_ms: number; text: string; speaker?: number; label?: string; ms?: number; gap?: boolean }

/** Who has spoken so far: the latest label of each speaker number, their talk time, and the ribbon's segments in order. */
export function talk(lines: Said[]) {
  const label = new Map<number, string>()
  const ms = new Map<number, number>()
  const segments: { speaker: number | null; weight: number }[] = []
  for (const l of lines) {
    if (l.speaker === undefined) continue
    label.set(l.speaker, l.label ?? `Speaker ${l.speaker + 1}`)
    ms.set(l.speaker, (ms.get(l.speaker) ?? 0) + (l.ms ?? 0))
    segments.push({ speaker: l.speaker, weight: Math.max(l.ms ?? 0, 1) })
  }
  const totals = [...label].sort((a, b) => a[0] - b[0]).map(([speaker, name]) => ({ speaker, label: name, ms: ms.get(speaker) ?? 0 }))
  return { totals, segments }
}
