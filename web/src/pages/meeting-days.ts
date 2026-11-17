import type { Meeting } from '../api'

const DAY = 86_400_000
const startOfDay = (ms: number) => { const d = new Date(ms); d.setHours(0, 0, 0, 0); return d.getTime() }

export const meetingTime = (m: Meeting) => m.started_at ?? m.created_at

/** "Today", "Yesterday", else weekday and date, in the viewer's time zone. */
export function dayLabel(ms: number, now = Date.now()): string {
  const diff = Math.round((startOfDay(now) - startOfDay(ms)) / DAY)
  if (diff === 0) return 'Today'
  if (diff === 1) return 'Yesterday'
  return new Date(ms).toLocaleDateString([], { weekday: 'long', day: 'numeric', month: 'long' })
}

/** Groups meetings by day, newest day first and newest meeting first within a day. */
export function groupByDay(meetings: Meeting[], now = Date.now()): { label: string; items: Meeting[] }[] {
  const sorted = [...meetings].sort((a, b) => meetingTime(b) - meetingTime(a))
  const groups: { label: string; items: Meeting[] }[] = []
  for (const m of sorted) {
    const label = dayLabel(meetingTime(m), now)
    const last = groups[groups.length - 1]
    // Same label twice in a row means the same day (labels differ across days, and the list is sorted).
    if (last && last.label === label) last.items.push(m)
    else groups.push({ label, items: [m] })
  }
  return groups
}

export const statusBadge: Record<string, { tone: 'good' | 'signal' | 'bad' | 'neutral'; label: string }> = {
  ready: { tone: 'good', label: 'Ready' },
  failed: { tone: 'bad', label: 'Failed' },
  uploaded: { tone: 'signal', label: 'Uploaded' },
  scheduled: { tone: 'neutral', label: 'Scheduled' },
  joining: { tone: 'signal', label: 'Joining' },
  recording: { tone: 'signal', label: 'Recording' },
  labelling: { tone: 'signal', label: 'Labelling speakers' },
  indexing: { tone: 'signal', label: 'Indexing' },
  transcribing: { tone: 'signal', label: 'Transcribing' },
  reporting: { tone: 'signal', label: 'Writing report' },
}
