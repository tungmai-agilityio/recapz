import { useCallback, useRef, useState, type ReactNode } from 'react'
import { ToastContext } from './toast-context'

/** Shows the latest message in a polite live region for 3 seconds. */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [msg, setMsg] = useState('')
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined)
  const show = useCallback((m: string) => {
    clearTimeout(timer.current)
    setMsg(m)
    timer.current = setTimeout(() => setMsg(''), 3000)
  }, [])
  return (
    <ToastContext.Provider value={show}>
      {children}
      <div role="status" aria-live="polite" className="pointer-events-none fixed bottom-4 left-1/2 -translate-x-1/2">
        {msg && <p className="rounded-md bg-ink px-3.5 py-2 text-sm text-paper shadow-lg">{msg}</p>}
      </div>
    </ToastContext.Provider>
  )
}
