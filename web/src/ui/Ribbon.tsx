import type { KeyboardEvent, MouseEvent } from 'react'

export interface RibbonSegment { start_ms: number; end_ms: number; speaker: number | null }

const colours = ['bg-speaker-1', 'bg-speaker-2', 'bg-speaker-3', 'bg-speaker-4', 'bg-speaker-5']
const speakerColour = (s: number | null) => (s === null ? 'bg-hair' : colours[s % colours.length])

/** Thin vertical bar in a speaker's colour, for transcript lines. */
export const SpeakerBar = ({ speaker }: { speaker: number | null }) => <span className={`rounded-sm ${speakerColour(speaker)}`} />

/** Speaker ribbon: one block per segment, width = its length, colour = speaker (`null` = unknown).
 *  Pass `onSeek` and `now` for the scrubber variant (chapter ticks, playhead, click and arrow keys seek). */
export function Ribbon({ segments, total, height = 14, ticks, now, onSeek, label = 'Speaker ribbon' }: {
  segments: RibbonSegment[]; total: number; height?: number
  ticks?: { ms: number; title: string }[]; now?: number; onSeek?: (ms: number) => void; label?: string
}) {
  const pct = (ms: number) => `${Math.min(100, Math.max(0, (ms / total) * 100))}%`
  const strip = (
    <div className="flex overflow-hidden rounded-[4px] bg-hair-2" style={{ height }} aria-hidden>
      {segments.map((s, i) => (
        <span key={i} className={speakerColour(s.speaker)} style={{ flexGrow: Math.max(s.end_ms - s.start_ms, 1), flexBasis: 0 }} />
      ))}
    </div>
  )
  if (!onSeek || now === undefined) return strip
  const click = (e: MouseEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect()
    if (r.width) onSeek(Math.round(Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)) * total))
  }
  const key = (e: KeyboardEvent) => {
    const d = e.key === 'ArrowRight' ? 5000 : e.key === 'ArrowLeft' ? -5000 : 0
    if (!d) return
    e.preventDefault()
    onSeek(Math.min(total, Math.max(0, now + d)))
  }
  return (
    <div role="slider" tabIndex={0} aria-label={label} aria-valuemin={0} aria-valuemax={Math.round(total / 1000)}
      aria-valuenow={Math.round(now / 1000)} onClick={click} onKeyDown={key}
      className="relative cursor-pointer select-none pt-[22px]">
      {ticks?.map((t) => (
        <span key={`${t.ms}-${t.title}`} title={t.title} style={{ left: pct(t.ms) }}
          className="pointer-events-none absolute top-0 hidden h-[50px] max-w-32 truncate whitespace-nowrap border-l border-hair pl-1 font-mono text-[0.64rem] text-mute sm:block">
          {t.title}
        </span>
      ))}
      {strip}
      <span data-testid="playhead" style={{ left: pct(now) }} className="pointer-events-none absolute -bottom-1 top-4 w-0.5 bg-signal">
        <span className="absolute -left-1 -top-1 size-2.5 rounded-full bg-signal" />
      </span>
    </div>
  )
}
