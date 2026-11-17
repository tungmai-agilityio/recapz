import type { KeyboardEvent, MouseEvent } from 'react'

const colours = ['bg-speaker-1', 'bg-speaker-2', 'bg-speaker-3', 'bg-speaker-4', 'bg-speaker-5']
const sizes = { list: 'h-1.5 w-[120px] rounded-[2px]', scrubber: 'h-7 w-full rounded', live: 'h-5 w-full rounded-[2px]' }
const colour = (s: number | null) => (s === null ? 'bg-hair' : colours[s % colours.length])

/** The speaker ribbon: a strip of who spoke when. `speaker` is 0-based (`null` = unknown); `weight` is the share of the width.
 *  With `onSeek`, `now` and `total` (ms) the scrubber becomes a slider: chapter `ticks`, amber playhead, click and arrow keys seek. */
export function Ribbon({ segments, variant = 'list', ticks, now, total, onSeek, label = 'Speaker ribbon' }: {
  segments: { speaker: number | null; weight: number }[]; variant?: keyof typeof sizes
  ticks?: { ms: number; title: string }[]; now?: number; total?: number; onSeek?: (ms: number) => void; label?: string
}) {
  const strip = (
    <div aria-hidden data-variant={variant} className={`flex gap-px overflow-hidden bg-hair-2 ${sizes[variant]}`}>
      {segments.map((s, i) => (
        <span key={i} data-speaker={s.speaker} style={{ flex: s.weight }}
          className={`block h-full ${colour(s.speaker)} ${variant === 'scrubber' ? 'opacity-90 hover:opacity-100' : ''}`} />
      ))}
    </div>
  )
  if (!onSeek || now === undefined || !total) return strip
  const pct = (ms: number) => `${Math.min(100, Math.max(0, (ms / total) * 100))}%`
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

/** Thin vertical bar in a speaker's colour, for transcript lines. */
export const SpeakerBar = ({ speaker }: { speaker: number | null }) => <span className={`rounded-sm ${colour(speaker)}`} />
