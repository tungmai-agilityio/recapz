import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, type ReactNode } from 'react'
import { Link, NavLink, useLocation, useNavigate } from 'react-router'
import { getMe, logout, Unauthorized } from './api'
import { ThemeSwitch } from './ThemeSwitch'
import { Badge } from './ui/Badge'
import { Face } from './ui/Face'
import { Menu, MenuItem } from './ui/Menu'

const groups: { label: string; items: { to: string; label: string; end?: boolean }[] }[] = [
  { label: 'Work', items: [{ to: '/', label: 'Meetings', end: true }, { to: '/ask', label: 'Ask' }] },
  { label: 'Workspace', items: [{ to: '/recording', label: 'Recording & privacy' }] },
]

function crumb(path: string) {
  if (path === '/recording') return <>Workspace / <b className="font-medium text-ink">Recording & privacy</b></>
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
  const navigate = useNavigate()
  const out = useMutation({ mutationFn: logout, onSuccess: () => qc.resetQueries() })

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'k') return
      e.preventDefault()
      navigate('/ask')
      setTimeout(() => document.querySelector<HTMLInputElement>('[aria-label="Question"]')?.focus())
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [navigate])

  if (me.error instanceof Unauthorized)
    return <div className="mx-auto max-w-xl px-4 py-12"><div className="mb-6 flex items-center gap-2.5"><Glyph /><span className="text-lg font-bold tracking-tight">Recapz</span></div>{children}</div>

  // ponytail: read at render, so a resize while the page is open only takes effect on the next render
  const narrow = window.matchMedia?.('(max-width: 900px)').matches
  const who = me.data?.name || me.data?.email || ''
  return (
    <div className="grid min-h-screen grid-cols-[208px_minmax(0,1fr)] max-[900px]:grid-cols-1 max-[900px]:grid-rows-[auto_1fr]">
      <nav aria-label="Screens"
        className="sticky top-0 flex h-screen flex-col gap-5 overflow-x-hidden overflow-y-auto border-r border-hair bg-rail px-2.5 py-4 text-rail-fg max-[900px]:h-auto max-[900px]:flex-row max-[900px]:flex-wrap max-[900px]:items-center max-[900px]:gap-1 max-[900px]:overflow-visible max-[900px]:border-r-0 max-[900px]:border-b max-[900px]:py-2">
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
          <div className="relative mt-auto border-t border-hair pt-2.5 max-[900px]:mt-0 max-[900px]:ml-auto max-[900px]:border-t-0 max-[900px]:pt-0">
            <Menu label="Account" side={narrow ? 'below' : 'above'} align={narrow ? 'right' : 'left'} className="min-w-0 [&>button]:w-full max-[900px]:[&>button]:w-auto" trigger={
              <span className="flex w-full min-w-0 items-center gap-2.5 rounded-md p-1 text-left hover:bg-nav-on-bg">
                <Face name={who} speaker={0} size={30} />
                <span className="min-w-0 flex-1 max-[900px]:hidden">
                  <span className="block truncate text-sm font-semibold text-rail-on">{me.data.name || me.data.email}</span>
                  {me.data.name && <span className="block truncate text-xs opacity-80">{me.data.email}</span>}
                </span>
              </span>
            }>
              <div className="mb-1 flex flex-col gap-0.5 border-b border-hair px-2.5 pt-2.5 pb-2">
                <b className="break-all">{me.data.name || me.data.email}</b>
                {me.data.name && <small className="break-all text-mute">{me.data.email}</small>}
                <span className="mt-1.5 self-start"><Badge>{me.data.role.charAt(0).toUpperCase() + me.data.role.slice(1)}</Badge></span>
              </div>
              <MenuItem onClick={() => out.mutate()}>Sign out</MenuItem>
            </Menu>
            {out.isError && <span role="alert" className="block pt-1 text-xs text-bad">Could not sign out.</span>}
          </div>
        )}
      </nav>
      <div className="min-w-0">
        <header className="sticky top-0 z-[6] flex flex-wrap items-center gap-3 border-b border-hair bg-sheet px-7 py-2.5 max-[900px]:static max-[900px]:px-4">
          <div className="min-w-0 truncate text-sm text-mute max-[900px]:hidden">{crumb(pathname)}</div>
          <Link to="/ask" className="ml-auto flex w-[300px] min-w-0 items-center gap-2 rounded-md border border-hair bg-sheet px-2.5 py-1.5 text-sm text-mute no-underline max-[900px]:order-first max-[900px]:ml-0 max-[900px]:w-auto max-[900px]:basis-full">
            <span aria-hidden>⌕</span><span>Ask about any meeting…</span>
            <kbd className="ml-auto rounded border border-hair px-1.5 font-mono text-[0.68rem] max-[900px]:hidden">{/Mac/.test(navigator.platform) ? '⌘K' : 'Ctrl K'}</kbd>
          </Link>
          <ThemeSwitch />
        </header>
        <main className="max-w-[1120px] px-7 pt-7 pb-16 max-[900px]:px-4 max-[900px]:pt-5">{children}</main>
      </div>
    </div>
  )
}
