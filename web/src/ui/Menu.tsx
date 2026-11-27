import { useEffect, useRef, useState, type ReactNode } from 'react'

/** Button that opens a `role=menu` panel; Esc or a click outside closes it, picking an item closes it too. */
export function Menu({ trigger, label, side = 'above', align = 'right', className = '', children }: { trigger: ReactNode; label: string; side?: 'above' | 'below'; align?: 'left' | 'right'; className?: string; children: ReactNode }) {
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
    <div ref={box} className={`relative ${className}`}>
      <button type="button" className="max-w-full" aria-haspopup="menu" aria-expanded={open} aria-label={label} onClick={() => setOpen(!open)}>{trigger}</button>
      {open && (
        <div role="menu" aria-label={label} onClick={() => setOpen(false)}
          className={`absolute ${align === 'left' ? 'left-0' : 'right-0'} z-40 flex ${side === 'below' ? 'top-full mt-1.5' : 'bottom-full mb-1.5'} min-w-48 max-w-[calc(100vw-1rem)] flex-col rounded-lg border border-hair bg-sheet p-1.5 text-ink shadow-xl`}>
          {children}
        </div>
      )}
    </div>
  )
}

export function MenuItem({ className = '', ...props }: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return <button type="button" role="menuitem" {...props}
    className={`rounded-md px-2.5 py-2 text-left text-sm hover:bg-hair-2 ${className}`} />
}

/** A `MenuItem` that is a plain link, e.g. a file download. The menu stays open on click: closing it would remove
 * the link before the browser follows it, and a link that is no longer in the page may not be followed. */
export function MenuLink({ className = '', onClick, ...props }: React.AnchorHTMLAttributes<HTMLAnchorElement>) {
  return <a role="menuitem" {...props} onClick={(e) => { e.stopPropagation(); onClick?.(e) }}
    className={`rounded-md px-2.5 py-2 text-left text-sm hover:bg-hair-2 ${className}`} />
}
