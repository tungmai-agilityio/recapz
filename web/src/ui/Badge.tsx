import type { ReactNode } from 'react'

const tones = {
  neutral: 'bg-hair-2 text-ink-2',
  good: 'bg-good-wash text-good',
  signal: 'bg-signal-wash text-signal-ink',
  bad: 'bg-bad-wash text-bad',
}

export function Badge({ tone = 'neutral', children }: { tone?: keyof typeof tones; children: ReactNode }) {
  return <span className={`inline-flex items-center gap-1 rounded px-1.5 py-px text-xs font-medium whitespace-nowrap ${tones[tone]}`}>{children}</span>
}
