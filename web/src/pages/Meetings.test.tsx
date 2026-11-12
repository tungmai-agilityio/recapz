import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { Meetings } from './Meetings'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const show = () =>
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter><Meetings /></MemoryRouter>
    </QueryClientProvider>,
  )

test('lists meetings from the API', async () => {
  const rows = [{ id: '1', title: 'Weekly sync', platform: 'upload', status: 'uploaded', started_at: null, created_at: 1 }]
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(rows))))
  show()
  expect(await screen.findByText('Weekly sync')).toBeTruthy()
})

test('shows sign-in when signed out', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 401 })))
  show()
  expect(await screen.findByText('Sign in with Google')).toBeTruthy()
})

test('dev login posts the email, then loads the meetings', async () => {
  const rows = [{ id: '1', title: 'After login', platform: 'upload', status: 'uploaded', started_at: null, created_at: 1 }]
  let signedIn = false
  const fetchMock = vi.fn(async (url: string) => {
    if (url === '/auth/dev-login') { signedIn = true; return new Response('') }
    return signedIn ? new Response(JSON.stringify(rows)) : new Response('', { status: 401 })
  })
  vi.stubGlobal('fetch', fetchMock)
  show()
  fireEvent.change(await screen.findByLabelText('Dev login (local only)'), { target: { value: ' me@example.com ' } })
  fireEvent.click(screen.getByRole('button', { name: 'Sign in' }))
  expect(await screen.findByText('After login')).toBeTruthy()
  const call = fetchMock.mock.calls.find((c) => c[0] === '/auth/dev-login') as unknown as [string, RequestInit]
  expect(JSON.parse(call[1].body as string)).toEqual({ email: 'me@example.com' })
})

test('dev login says so when the server rejects it', async () => {
  vi.stubGlobal('fetch', vi.fn(async (url: string) => new Response('', { status: url === '/auth/dev-login' ? 403 : 401 })))
  show()
  fireEvent.change(await screen.findByLabelText('Dev login (local only)'), { target: { value: 'x@example.com' } })
  fireEvent.click(screen.getByRole('button', { name: 'Sign in' }))
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('dev_login = true'))
})
