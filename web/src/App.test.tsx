import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import App from './App'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

test('renders the app shell with the Meetings page', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('[]')))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter><App /></MemoryRouter>
    </QueryClientProvider>,
  )
  expect(screen.getByText('Recapz')).toBeTruthy()
  expect(await screen.findByText(/No meetings yet/)).toBeTruthy()
})

test('Sign out posts to /auth/logout with the csrf header and returns to the sign-in state', async () => {
  document.cookie = 'recapz_csrf=tok'
  let out = false
  const fetchMock = vi.fn(async (url: string) => {
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
  const init = fetchMock.mock.calls.find((c) => c[0] === '/auth/logout')![1] as unknown as RequestInit
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
