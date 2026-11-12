import { useEffect, useId, useRef, type ReactNode } from 'react'
import { Button } from './Button'

/** Modal confirm: focus starts on Cancel; Esc or Cancel closes. `error` shows inside so a failed action stays retryable. */
export function ConfirmDialog({ title, children, confirmLabel, busy, error, onConfirm, onCancel }: {
  title: string
  children: ReactNode
  confirmLabel: string
  busy?: boolean
  error?: string
  onConfirm: () => void
  onCancel: () => void
}) {
  const [tid, did] = [useId(), useId()]
  const cancel = useRef<HTMLButtonElement>(null)
  const close = useRef(onCancel)
  // Latest handler without re-running the focus effect on every render.
  useEffect(() => { close.current = onCancel })
  useEffect(() => {
    cancel.current?.focus()
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') close.current() }
    document.addEventListener('keydown', esc)
    return () => document.removeEventListener('keydown', esc)
  }, [])
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-ink/40 p-4">
      <div role="alertdialog" aria-modal="true" aria-labelledby={tid} aria-describedby={did}
        className="w-full max-w-sm rounded-lg border border-hair bg-sheet p-5 text-ink shadow-xl">
        <h2 id={tid} className="text-lg font-semibold">{title}</h2>
        <p id={did} className="mt-2 text-sm text-ink-2">{children}</p>
        {error && <p role="alert" className="mt-3 text-sm text-bad">{error}</p>}
        <div className="mt-5 flex justify-end gap-2">
          <Button ref={cancel} onClick={onCancel}>Cancel</Button>
          <Button variant="danger" onClick={onConfirm} disabled={busy}>{confirmLabel}</Button>
        </div>
      </div>
    </div>
  )
}
