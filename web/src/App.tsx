import { useQuery, useQueryClient } from '@tanstack/react-query'
import { NavLink, Route, Routes } from 'react-router'
import { getMe, logout, Unauthorized } from './api'
import { ThemeSwitch } from './ThemeSwitch'
import { Ask } from './pages/Ask'
import { Meetings } from './pages/Meetings'
import { Report } from './pages/Report'
import { Button } from './ui/Button'

export default function App() {
  const qc = useQueryClient()
  const me = useQuery({ queryKey: ['me'], queryFn: getMe, retry: (n, e) => !(e instanceof Unauthorized) && n < 2 })
  const signOut = async () => { await logout(); await qc.resetQueries() }
  return (
    <div className="mx-auto max-w-3xl px-4 py-8">
      <header className="mb-8 flex flex-wrap items-center gap-3">
        <span aria-hidden className="flex h-4 items-end gap-0.5">
          <i className="block h-2 w-1 bg-speaker-1" />
          <i className="block h-4 w-1 bg-speaker-2" />
          <i className="block h-2.5 w-1 bg-speaker-3" />
          <i className="block h-4 w-1 bg-signal" />
        </span>
        <span className="text-lg font-bold tracking-tight">Recapz</span>
        <nav className="ml-6 text-sm">
          <NavLink to="/" className="font-medium text-primary">Meetings</NavLink>
          <NavLink to="/ask" className="ml-4 font-medium text-primary">Ask</NavLink>
        </nav>
        <ThemeSwitch />
        {me.data && <Button variant="ghost" onClick={signOut}>Sign out</Button>}
      </header>
      <main>
        <Routes>
          <Route path="/" element={<Meetings />} />
          <Route path="/ask" element={<Ask />} />
          <Route path="/meetings/:id" element={<Report />} />
        </Routes>
      </main>
    </div>
  )
}
