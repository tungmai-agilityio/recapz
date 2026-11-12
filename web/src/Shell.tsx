import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { NavLink, useLocation } from 'react-router'
import { getMe, logout, Unauthorized } from './api'
import { ThemeSwitch } from './ThemeSwitch'
import { Button } from './ui/Button'
import { Face } from './ui/Face'

const groups: { label: string; items: { to: string; label: string; end?: boolean }[] }[] = [
  { label: 'Work', items: [{ to: '/', label: 'Meetings', end: true }, { to: '/ask', label: 'Ask' }] },
]

function crumb(path: string) {
  if (path === '/ask') return <b className="font-medium text-ink">Ask</b>
  if (path.startsWith('/meetings/')) return <>Meetings / <b className="font-medium text-ink">Meeting report</b></>
  return <b className="font-medium text-ink">Meetings</b>
}

const Glyph = () => (
  <span aria-hidden className="flex h-[18px] items-center gap-0.5">
    <i className="block h-2 w-1 rounded-[1px] bg-speaker-1" />
    <i className="block h-4 w-1 rounded-[1px] bg-speaker-2" />
    <i className="block h-[11px] w-1 rounded-[1px] bg-speaker-3" />
    <i className="block h-[18px] w-1 rounded-[1px] bg-signal" />
  </span>
)

/** Rail, top bar and page frame. A signed-out visitor gets just the page (the sign-in card). */
export function Shell({ children }: { children: ReactNode }) {
  const qc = useQueryClient()
  const { pathname } = useLocation()
  const me = useQuery({ queryKey: ['me'], queryFn: getMe, retry: (n, e) => !(e instanceof Unauthorized) && n < 2 })
  const out = useMutation({ mutationFn: logout, onSuccess: () => qc.resetQueries() })

  if (me.error instanceof Unauthorized)
    return <div className="mx-auto max-w-xl px-4 py-12"><div className="mb-6 flex items-center gap-2.5"><Glyph /><span className="text-lg font-bold tracking-tight">Recapz</span></div>{children}</div>

  const who = me.data?.name || me.data?.email || ''
  return (
    <div className="grid min-h-screen grid-cols-[208px_minmax(0,1fr)] max-[900px]:grid-cols-1 max-[900px]:grid-rows-[auto_1fr]">
      <nav aria-label="Screens"
        className="sticky top-0 flex h-screen flex-col gap-5 overflow-y-auto border-r border-hair bg-rail px-2.5 py-4 text-rail-fg max-[900px]:h-auto max-[900px]:flex-row max-[900px]:items-center max-[900px]:gap-1 max-[900px]:overflow-x-auto max-[900px]:border-r-0 max-[900px]:border-b max-[900px]:py-2">
        <NavLink to="/" end aria-label="Recapz home" className="flex items-center gap-2 px-2 font-bold tracking-tight text-rail-on no-underline">
          <Glyph /><span>Recapz</span>
        </NavLink>
        {groups.map((g) => (
          <div key={g.label} className="flex flex-col gap-px max-[900px]:flex-row">
            <div className="px-2.5 pb-1.5 text-[0.66rem] tracking-[0.12em] uppercase opacity-60 max-[900px]:hidden">{g.label}</div>
            {g.items.map((i) => (
              <NavLink key={i.to} to={i.to} end={i.end}
                className={({ isActive }) => `relative rounded-md px-2.5 py-1.5 text-sm font-medium no-underline hover:text-rail-on ${isActive ? 'bg-nav-on-bg text-nav-on-fg before:absolute before:top-2 before:bottom-2 before:-left-2.5 before:w-[3px] before:rounded-r-sm before:bg-nav-marker max-[900px]:before:hidden' : ''}`}>
                {i.label}
              </NavLink>
            ))}
          </div>
        ))}
        {me.data && (
          <div className="mt-auto flex items-center gap-2.5 border-t border-hair px-1 pt-2.5 max-[900px]:mt-0 max-[900px]:ml-auto max-[900px]:border-t-0 max-[900px]:pt-0">
            <Face name={who} speaker={0} size={30} />
            <span className="min-w-0 max-[900px]:hidden">
              <span className="block truncate text-sm font-semibold text-rail-on">{me.data.name || me.data.email}</span>
              {me.data.name && <span className="block max-w-[120px] truncate text-xs opacity-80">{me.data.email}</span>}
            </span>
            <Button variant="ghost" className="text-rail-fg! hover:bg-nav-on-bg!" onClick={() => out.mutate()}>Sign out</Button>
            {out.isError && <span role="alert" className="text-xs text-bad">Could not sign out ({out.error.message.trim().slice(0, 80)}). Try again.</span>}
          </div>
        )}
      </nav>
      <div className="min-w-0">
        <header className="sticky top-0 z-[6] flex flex-wrap items-center gap-3 border-b border-hair bg-sheet px-7 py-2.5 max-[900px]:static max-[900px]:px-4">
          <div className="min-w-0 truncate text-sm text-mute max-[900px]:hidden">{crumb(pathname)}</div>
          <ThemeSwitch />
        </header>
        <main className="max-w-[1120px] px-7 pt-7 pb-16 max-[900px]:px-4 max-[900px]:pt-5">{children}</main>
      </div>
    </div>
  )
}
