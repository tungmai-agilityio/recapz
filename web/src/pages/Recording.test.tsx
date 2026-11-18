import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { ToastProvider } from '../ui/Toast'
import { Recording } from './Recording'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

function show(url: string, status: object, calls: string[] = [], failDelete = false) {
  vi.stubGlobal('fetch', vi.fn(async (u: string, init?: RequestInit) => {
    calls.push(`${init?.method ?? 'GET'} ${u}`)
    if (init?.method === 'DELETE') return failDelete ? new Response('no', { status: 500 }) : new Response(null, { status: 204 })
    return new Response(JSON.stringify(status))
  }))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ToastProvider><MemoryRouter initialEntries={[url]}><Recording /></MemoryRouter></ToastProvider>
    </QueryClientProvider>,
  )
}

test('not connected: a Connect link to the grant, no Disconnect', async () => {
  show('/recording', { connected: false, needs_reconnect: false })
  expect((await screen.findByRole('link', { name: 'Connect Google Calendar' })).getAttribute('href')).toBe('/auth/calendar')
  expect(screen.getByText('Not connected')).toBeTruthy()
  expect(screen.queryByRole('button', { name: 'Disconnect' })).toBeNull()
})

test('connected: a Connected badge and Disconnect only; confirming calls DELETE and toasts', async () => {
  const calls: string[] = []
  show('/recording', { connected: true, needs_reconnect: false }, calls)
  expect(await screen.findByText('Connected')).toBeTruthy()
  expect(screen.queryByRole('link', { name: /Connect|Reconnect/ })).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }))
  expect(calls.filter((c) => c.startsWith('DELETE'))).toEqual([])
  fireEvent.click(screen.getAllByRole('button', { name: 'Disconnect' }).at(-1)!)
  await waitFor(() => expect(calls).toContain('DELETE /api/calendar'))
  expect(await screen.findByText('Google Calendar disconnected')).toBeTruthy()
  expect(screen.queryByRole('alertdialog')).toBeNull()
})

test('a failed disconnect keeps the dialog open and says so', async () => {
  show('/recording', { connected: true, needs_reconnect: false }, [], true)
  fireEvent.click(await screen.findByRole('button', { name: 'Disconnect' }))
  fireEvent.click(screen.getAllByRole('button', { name: 'Disconnect' }).at(-1)!)
  expect(await screen.findByText('Could not disconnect. Try again.')).toBeTruthy()
  expect(screen.getByRole('alertdialog')).toBeTruthy()
})

test('needs reconnect: a Reconnect link and a bad badge, and the stale connection can still be removed', async () => {
  show('/recording', { connected: true, needs_reconnect: true })
  expect((await screen.findByRole('link', { name: 'Reconnect Google Calendar' })).getAttribute('href')).toBe('/auth/calendar')
  expect(screen.getByText('Reconnect needed').className).toContain('text-bad')
  expect(screen.getByRole('button', { name: 'Disconnect' })).toBeTruthy()
})

test.each([
  ['connected', 'Google Calendar is connected.', 'status'],
  ['denied', 'Calendar access was not granted, so nothing was connected.', 'alert'],
  ['scope_missing', 'The calendar box was unticked on the Google screen, so nothing was connected.', 'alert'],
  ['wrong_account', 'That Google account is not the one you signed in with. Pick the same account and try again.', 'alert'],
  ['error', 'Google Calendar could not be connected. Try again.', 'alert'],
])('the %s outcome in the address shows its message', async (code, text, role) => {
  show(`/recording?calendar=${code}`, { connected: false, needs_reconnect: false })
  const msg = await screen.findByText(text)
  expect(msg.parentElement?.getAttribute('role') ?? 'status').toBe(role)
})

test('an unknown outcome shows nothing, and Dismiss removes a shown one', async () => {
  show('/recording?calendar=%3Cb%3E', { connected: false, needs_reconnect: false })
  await screen.findByText('Not connected')
  expect(screen.queryByRole('alert')).toBeNull()
  cleanup()
  show('/recording?calendar=connected', { connected: true, needs_reconnect: false })
  fireEvent.click(await screen.findByRole('button', { name: 'Dismiss' }))
  expect(screen.queryByText('Google Calendar is connected.')).toBeNull()
})

test('names the speech provider from /api/me', async () => {
  show('/recording', { email: 'a@b.c', name: null, role: 'admin', speech_provider: 'Deepgram', connected: false, needs_reconnect: false })
  expect(await screen.findByText('Deepgram')).toBeTruthy()
  expect(screen.getByText(/is sent to/).textContent).toMatch(/Audio is sent to Deepgram for transcription/)
})
