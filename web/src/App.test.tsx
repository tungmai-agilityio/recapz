import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import App from './App'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

function mount(path: string, me: Response, calls: string[] = [], logoutStatus = 200) {
  let loggedOut = false
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    calls.push(`${init?.method ?? 'GET'} ${url}`)
    if (url === '/auth/logout') { loggedOut = logoutStatus === 200; return new Response('', { status: logoutStatus }) }
    if (loggedOut) return new Response('', { status: 401 })
    if (url === '/api/me') return me.clone()
    return me.status === 401 ? new Response('', { status: 401 }) : new Response('[]')
  }))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter initialEntries={[path]}><App /></MemoryRouter>
    </QueryClientProvider>,
  )
}

const signedIn = () => new Response(JSON.stringify({ email: 'ana@example.com', name: 'Ana', role: 'admin' }))

test('signed-in shell shows the rail, the account and the Meetings page', async () => {
  mount('/', signedIn())
  expect(await screen.findByText('No meetings yet. Upload a recording to get started.')).toBeTruthy()
  const rail = screen.getByRole('navigation', { name: 'Screens' })
  expect(rail.textContent).toContain('Work')
  expect(screen.getByRole('link', { name: 'Meetings' }).getAttribute('aria-current')).toBe('page')
  expect(screen.getByRole('link', { name: 'Ask' }).getAttribute('aria-current')).toBeNull()
  expect(screen.getByRole('link', { name: 'Recapz home' }).getAttribute('aria-current')).toBe('page')
  expect(await screen.findByText('ana@example.com')).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Sign out' })).toBeTruthy()
})

test('the active rail item and breadcrumb follow the route', async () => {
  mount('/ask', signedIn())
  expect((await screen.findByRole('link', { name: 'Ask' })).getAttribute('aria-current')).toBe('page')
  expect(screen.getByRole('link', { name: 'Meetings' }).getAttribute('aria-current')).toBeNull()
  expect(screen.getByRole('link', { name: 'Recapz home' }).getAttribute('aria-current')).toBeNull()
  expect(document.querySelector('header')?.textContent).toContain('Ask')
  expect(document.querySelector('header')?.textContent).not.toContain('Meetings')
})

test('Sign out posts to /auth/logout and returns to the sign-in card', async () => {
  const calls: string[] = []
  mount('/', signedIn(), calls)
  fireEvent.click(await screen.findByRole('button', { name: 'Sign out' }))
  await waitFor(() => expect(calls).toContain('POST /auth/logout'))
  expect(await screen.findByText('Sign in to Recapz')).toBeTruthy()
  expect(screen.queryByRole('navigation', { name: 'Screens' })).toBeNull()
})

test('a failed sign-out keeps the shell and says so', async () => {
  mount('/', signedIn(), [], 403)
  fireEvent.click(await screen.findByRole('button', { name: 'Sign out' }))
  expect((await screen.findByRole('alert')).textContent).toBe('Could not sign out (403). Try again.')
  expect(screen.getByRole('navigation', { name: 'Screens' })).toBeTruthy()
})

test('the report route shows its breadcrumb', async () => {
  mount('/meetings/m1', signedIn())
  await screen.findByRole('navigation', { name: 'Screens' })
  expect(document.querySelector('header')?.textContent).toContain('Meetings / Meeting report')
})

test('a 401 shows only the sign-in card, with no rail', async () => {
  mount('/', new Response('', { status: 401 }))
  expect(await screen.findByText('Sign in to Recapz')).toBeTruthy()
  expect(screen.queryByRole('navigation', { name: 'Screens' })).toBeNull()
})

test('Sign out posts to /auth/logout with the csrf header and returns to the sign-in state', async () => {
  document.cookie = 'recapz_csrf=tok'
  let out = false
  const fetchMock = vi.fn(async (url: string, _init?: RequestInit) => {
    if (url === '/auth/logout') { out = true; return new Response('') }
    if (out) return new Response('', { status: 401 })
    return url === '/api/me' ? new Response('{"email":"a@b.c","name":null,"role":"admin"}') : new Response('[]')
  })
  vi.stubGlobal('fetch', fetchMock)
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter><App /></MemoryRouter>
    </QueryClientProvider>,
  )
  fireEvent.click(await screen.findByRole('button', { name: 'Sign out' }))
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Sign out' })).toBeNull())
  expect(await screen.findByText('Sign in to Recapz')).toBeTruthy()
  const init = fetchMock.mock.calls.find((c) => c[0] === '/auth/logout')![1] as RequestInit
  expect(init.method).toBe('POST')
  expect(init.headers).toEqual({ 'x-csrf-token': 'tok' })
})

test('a failed logout keeps the session and shows an error', async () => {
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    if (url === '/auth/logout') return new Response('bad csrf token', { status: 403 })
    return url === '/api/me' ? new Response('{"email":"a@b.c","name":null,"role":"admin"}') : new Response('[]')
  }))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter><App /></MemoryRouter>
    </QueryClientProvider>,
  )
  fireEvent.click(await screen.findByRole('button', { name: 'Sign out' }))
  expect((await screen.findByRole('alert')).textContent).toBe('Could not sign out (403 bad csrf token). Try again.')
  expect(screen.getByRole('button', { name: 'Sign out' })).toBeTruthy()
  expect(screen.queryByText('Sign in to Recapz')).toBeNull()
})
