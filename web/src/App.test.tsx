import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import App from './App'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

function mount(path: string, me: Response, calls: string[] = []) {
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    calls.push(`${init?.method ?? 'GET'} ${url}`)
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
  expect(await screen.findByText('ana@example.com')).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Sign out' })).toBeTruthy()
})

test('the active rail item and breadcrumb follow the route', async () => {
  mount('/ask', signedIn())
  expect((await screen.findByRole('link', { name: 'Ask' })).getAttribute('aria-current')).toBe('page')
  expect(screen.getByRole('link', { name: 'Meetings' }).getAttribute('aria-current')).toBeNull()
})

test('Sign out posts to /auth/logout', async () => {
  const calls: string[] = []
  mount('/', signedIn(), calls)
  fireEvent.click(await screen.findByRole('button', { name: 'Sign out' }))
  await waitFor(() => expect(calls).toContain('POST /auth/logout'))
})

test('a 401 shows only the sign-in card, with no rail', async () => {
  mount('/', new Response('', { status: 401 }))
  expect(await screen.findByText('Sign in to Recapz')).toBeTruthy()
  expect(screen.queryByRole('navigation', { name: 'Screens' })).toBeNull()
})
