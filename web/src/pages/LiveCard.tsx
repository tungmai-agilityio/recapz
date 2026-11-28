import { useEffect, useState } from 'react'
import { Link } from 'react-router'
import type { Meeting } from '../api'
import { clock } from '../clock'
import { Ribbon } from '../ui/Ribbon'
import { Badge } from '../ui/Badge'

/** A call being recorded right now, with the time since it was admitted and a link to its live view. */
export function LiveCard({ m }: { m: Meeting }) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(t)
  }, [])
  const segments = (m.live_speakers ?? []).map((s) => ({ speaker: s.speaker, weight: Math.max(s.ms, 1) }))
  return (
    <div className="frame flex flex-wrap items-center gap-x-3 gap-y-2" data-testid="live-card">
      <Badge tone="signal">Recording</Badge>
      <div className="min-w-0 flex-1">
        <b className="block truncate font-semibold tracking-tight">{m.title}</b>
        <span className="text-xs text-mute">{m.platform}{m.started_at ? <> · <time className="font-mono">{clock(Math.max(0, now - m.started_at))}</time></> : null}</span>
      </div>
      {segments.length > 0 && <div className="w-full"><Ribbon segments={segments} variant="card" label="Live speaker ribbon" /></div>}
      <Link to={`/meetings/${m.id}/live`} className="inline-flex items-center gap-2 rounded-md border border-hair bg-sheet px-3 py-1.5 text-sm font-medium hover:border-ink-2">Open live view</Link>
    </div>
  )
}
