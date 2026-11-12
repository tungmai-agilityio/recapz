import { useEffect, useRef, useState, type ReactNode } from 'react'

/** Button that opens a `role=menu` panel; Esc or a click outside closes it, picking an item closes it too. */
export function Menu({ trigger, label, children }: { trigger: ReactNode; label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const out = (e: MouseEvent) => { if (!box.current?.contains(e.target as Node)) setOpen(false) }
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') setOpen(false) }
    document.addEventListener('mousedown', out)
    document.addEventListener('keydown', esc)
    return () => { document.removeEventListener('mousedown', out); document.removeEventListener('keydown', esc) }
  }, [open])
  return (
    <div ref={box} className="relative">
      <button type="button" aria-haspopup="menu" aria-expanded={open} aria-label={label} onClick={() => setOpen(!open)}>{trigger}</button>
      {open && (
        <div role="menu" aria-label={label} onClick={() => setOpen(false)}
          className="absolute right-0 bottom-full z-40 mb-1.5 flex min-w-48 flex-col rounded-lg border border-hair bg-sheet p-1.5 text-ink shadow-xl">
          {children}
        </div>
      )}
    </div>
  )
}

export function MenuItem(props: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return <button type="button" role="menuitem" {...props}
    className="rounded-md px-2.5 py-2 text-left text-sm hover:bg-hair-2" />
}
