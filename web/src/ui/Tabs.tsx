import type { KeyboardEvent } from 'react'

export interface TabItem { value: string; label: string }

/** Tab strip only; the caller renders the matching `role="tabpanel"`. Left and Right move between tabs. */
export function Tabs({ label, tabs, value, onChange }:
  { label: string; tabs: TabItem[]; value: string; onChange: (v: string) => void }) {
  const move = (e: KeyboardEvent, i: number) => {
    const d = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0
    if (!d) return
    const next = tabs[(i + d + tabs.length) % tabs.length]
    onChange(next.value)
    const buttons = e.currentTarget.parentElement?.querySelectorAll('button')
    buttons?.[tabs.indexOf(next)].focus()
  }
  return (
    <div role="tablist" aria-label={label} className="flex gap-4 border-b border-hair">
      {tabs.map((t, i) => (
        <button key={t.value} type="button" role="tab" aria-selected={t.value === value}
          tabIndex={t.value === value ? 0 : -1} onClick={() => onChange(t.value)} onKeyDown={(e) => move(e, i)}
          className={`-mb-px border-b-2 py-2 text-sm font-medium ${t.value === value ? 'border-primary text-primary' : 'border-transparent text-mute'}`}>
          {t.label}
        </button>
      ))}
    </div>
  )
}
