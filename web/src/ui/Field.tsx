import { useId, type ReactNode } from 'react'

export const inputClass = 'rounded-md border border-hair bg-sheet px-2.5 py-2 text-sm'

type Control = { id: string; 'aria-describedby'?: string; 'aria-invalid'?: true }

/** Label, hint and error around one control; `children` receives the props that wire them together. */
export function Field({ label, hint, error, children }:
  { label: string; hint?: string; error?: string; children: (props: Control) => ReactNode }) {
  const id = useId()
  const note = error ? `${id}-err` : hint ? `${id}-hint` : undefined
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="text-sm font-medium">{label}</label>
      {children({ id, 'aria-describedby': note, 'aria-invalid': error ? true : undefined })}
      {error && <p id={note} role="alert" className="text-xs text-bad">{error}</p>}
      {!error && hint && <p id={note} className="text-xs text-mute">{hint}</p>}
    </div>
  )
}
