import { NavLink, Route, Routes } from 'react-router'
import { ThemeSwitch } from './ThemeSwitch'
import { Ask } from './pages/Ask'
import { Meetings } from './pages/Meetings'
import { Report } from './pages/Report'

export default function App() {
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
