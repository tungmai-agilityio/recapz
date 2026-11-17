const colours = ['bg-speaker-1', 'bg-speaker-2', 'bg-speaker-3', 'bg-speaker-4', 'bg-speaker-5']
const sizes = { list: 'h-1.5 w-[120px] rounded-[2px]', scrubber: 'h-7 w-full rounded', live: 'h-5 w-full rounded-[2px]' }

/** The speaker ribbon: a strip of who spoke when. `speaker` is 0-based; `weight` is the share of the width. */
export function Ribbon({ segments, variant = 'list' }: { segments: { speaker: number; weight: number }[]; variant?: keyof typeof sizes }) {
  return (
    <div aria-hidden data-variant={variant} className={`flex gap-px overflow-hidden bg-hair-2 ${sizes[variant]}`}>
      {segments.map((s, i) => (
        <span key={i} data-speaker={s.speaker} style={{ flex: s.weight }}
          className={`block h-full ${colours[s.speaker % colours.length]} ${variant === 'scrubber' ? 'opacity-90 hover:opacity-100' : ''}`} />
      ))}
    </div>
  )
}
