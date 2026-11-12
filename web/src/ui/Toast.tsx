import { useCallback, useState, type ReactNode } from 'react'
import { ToastContext } from './toast-context'

/** Shows the latest message in a polite live region for 3 seconds. */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [msg, setMsg] = useState('')
  const show = useCallback((m: string) => {
    setMsg(m)
    setTimeout(() => setMsg((cur) => (cur === m ? '' : cur)), 3000)
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
