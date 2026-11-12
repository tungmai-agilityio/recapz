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

test('deleting from the list asks first, then calls DELETE for that row and refreshes', async () => {
  let rows = [
    { id: '1', title: 'Weekly sync', platform: 'upload', status: 'ready', started_at: null, created_at: 1 },
    { id: '2', title: 'Design review', platform: 'upload', status: 'ready', started_at: null, created_at: 2 },
  ]
  const f = vi.fn(async (_u: string, init?: RequestInit) => {
    if (init?.method === 'DELETE') { rows = rows.filter((r) => r.id !== '2'); return new Response(null, { status: 204 }) }
    return new Response(JSON.stringify(rows))
  })
  vi.stubGlobal('fetch', f)
  show()
  fireEvent.click(await screen.findByRole('button', { name: 'Delete Design review' }))
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(f.mock.calls.filter(([, i]) => i?.method === 'DELETE')).toHaveLength(0)
  fireEvent.click(screen.getByRole('button', { name: 'Delete Design review' }))
  fireEvent.click(screen.getByRole('button', { name: 'Delete meeting' }))
  await waitFor(() => expect(screen.queryByText('Design review')).toBeNull())
  const dels = f.mock.calls.filter(([, i]) => i?.method === 'DELETE')
  expect(dels).toHaveLength(1)
  expect(dels[0][0]).toBe('/api/meetings/2')
  expect(screen.getByText('Weekly sync')).toBeTruthy()
  expect(screen.queryByRole('alertdialog')).toBeNull()
})

test('a refused delete keeps the dialog open with the server message', async () => {
  const rows = [{ id: '1', title: 'Weekly sync', platform: 'upload', status: 'ready', started_at: null, created_at: 1 }]
  vi.stubGlobal('fetch', vi.fn(async (_u: string, init?: RequestInit) =>
    init?.method === 'DELETE' ? new Response('only the owner or an admin can delete', { status: 403 }) : new Response(JSON.stringify(rows))))
  show()
  fireEvent.click(await screen.findByRole('button', { name: 'Delete Weekly sync' }))
  fireEvent.click(screen.getByRole('button', { name: 'Delete meeting' }))
  expect((await screen.findByRole('alert')).textContent).toBe('only the owner or an admin can delete')
  expect(screen.getByRole('alertdialog')).toBeTruthy()
})
