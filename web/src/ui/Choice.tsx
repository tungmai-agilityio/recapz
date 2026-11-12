import { useId } from 'react'

export interface ChoiceOption { value: string; label: string; hint?: string }

/** Radio rows in one framed box. */
export function Choice({ label, options, value, onChange }:
  { label: string; options: ChoiceOption[]; value: string; onChange: (v: string) => void }) {
  const name = useId()
  return (
    <div role="radiogroup" aria-label={label}
      className="flex flex-col divide-y divide-hair overflow-hidden rounded-lg border border-hair bg-sheet">
      {options.map((o) => (
        <label key={o.value}
          className={`grid cursor-pointer grid-cols-[18px_1fr] gap-2.5 px-3.5 py-3 ${value === o.value ? 'bg-hair-2' : ''}`}>
          <input type="radio" name={name} value={o.value} checked={value === o.value}
            onChange={() => onChange(o.value)} className="mt-1 accent-primary" />
          <span>
            {o.label}
            {o.hint && <small className="block text-sm text-mute">{o.hint}</small>}
          </span>
        </label>
      ))}
    </div>
  )
}
