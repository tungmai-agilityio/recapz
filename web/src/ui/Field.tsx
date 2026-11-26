import { useId, type ReactNode } from 'react'

/** Same height as `Button` (py-1.5, text-sm) so a control and a button on one row line up. */
export const inputClass = 'rounded-md border border-hair bg-sheet px-2.5 py-1.5 text-sm disabled:opacity-50'

/** Native file input: its button looks like a `Button`. */
export const fileClass = 'max-w-full text-[.8rem] text-mute file:mr-3 file:cursor-pointer file:rounded-md file:border file:border-hair file:bg-sheet file:px-3 file:py-1.5 file:text-sm file:font-medium file:text-ink hover:file:border-ink-2'

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
