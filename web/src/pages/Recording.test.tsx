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

test('names no speech provider', async () => {
  show('/recording', { email: 'a@b.c', name: null, role: 'admin', connected: false, needs_reconnect: false })
  expect(await screen.findByText('Where audio goes')).toBeTruthy()
  expect(document.body.textContent).not.toMatch(/is sent to|Deepgram/)
})

test('names where recordings are kept and the retention period', async () => {
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(
    u === '/api/me' ? { email: 'a@b.c', name: null, role: 'admin', recording_storage: 'object_storage' }
      : u === '/api/settings/audio-retention' ? { days: 90 } : { connected: false, needs_reconnect: false },
  ))))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ToastProvider><MemoryRouter initialEntries={['/recording']}><Recording /></MemoryRouter></ToastProvider>
    </QueryClientProvider>,
  )
  expect((await screen.findByText(/Finished recordings are kept/)).textContent).toBe(
    'Finished recordings are kept in private object storage and their audio is deleted after 90 days. Transcripts and reports stay.',
  )
})

function showAs(role: string, usage: object) {
  const calls: string[] = []
  vi.stubGlobal('fetch', vi.fn(async (u: string) => {
    calls.push(u)
    return new Response(JSON.stringify(
      u === '/api/me' ? { email: 'a@b.c', name: null, role }
        : u === '/api/usage' ? usage : { connected: false, needs_reconnect: false },
    ))
  }))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ToastProvider><MemoryRouter initialEntries={['/recording']}><Recording /></MemoryRouter></ToastProvider>
    </QueryClientProvider>,
  )
  return calls
}

test("admins see this month's cost, the cap alert and the cost per meeting", async () => {
  showAs('admin', {
    speech_micro_usd: 546_000, llm_micro_usd: 4_000, total_micro_usd: 550_000, cap_micro_usd: 500_000, paused: true,
    meetings: [{ id: 'm1', title: 'Weekly sync', speech_micro_usd: 468_000, llm_micro_usd: 2_000 }],
  })
  expect(await screen.findByText('Usage this month')).toBeTruthy()
  expect(screen.getByRole('alert').textContent).toMatch(/monthly cap is reached/)
  expect(screen.getByText(/of \$0\.50 cap/).textContent).toBe('$0.55 of $0.50 cap · speech $0.55 · language model $0.0040')
  expect(screen.getByRole('list', { name: 'Cost per meeting' }).textContent).toBe('Weekly sync$0.47')
})

test('members never ask for usage and see no usage card', async () => {
  const calls = showAs('member', {})
  await screen.findByText('Where audio goes')
  await waitFor(() => expect(calls).toContain('/api/me'))
  expect(calls).not.toContain('/api/usage')
  expect(screen.queryByText('Usage this month')).toBeNull()
})

function retention(role: string, put?: Response) {
  const calls: { m: string; u: string; body?: string }[] = []
  vi.stubGlobal('fetch', vi.fn(async (u: string, init?: RequestInit) => {
    calls.push({ m: init?.method ?? 'GET', u, body: init?.body as string | undefined })
    if (init?.method === 'PUT') return put ?? new Response(JSON.stringify({ days: 180 }))
    return new Response(JSON.stringify(
      u === '/api/me' ? { email: 'a@b.c', name: null, role, recording_storage: 'local_disk' }
        : u === '/api/settings/audio-retention' ? { days: 90 }
          : u === '/api/usage' ? { speech_micro_usd: 0, llm_micro_usd: 0, total_micro_usd: 0, cap_micro_usd: 0, paused: false, meetings: [] }
            : { connected: false, needs_reconnect: false },
    ))
  }))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ToastProvider><MemoryRouter initialEntries={['/recording']}><Recording /></MemoryRouter></ToastProvider>
    </QueryClientProvider>,
  )
  return calls
}

test('admin picks a retention period: PUT carries the days and a toast says saved', async () => {
  const calls = retention('admin')
  const sel = (await screen.findByLabelText('Delete audio after')) as HTMLSelectElement
  expect(sel.value).toBe('90')
  fireEvent.change(sel, { target: { value: '180' } })
  await waitFor(() => expect(calls.filter((c) => c.m === 'PUT').map((c) => c.body)).toEqual(['{"days":180}']))
  expect(await screen.findByText('Audio retention saved')).toBeTruthy()
})

test('a refused retention change shows the server message', async () => {
  retention('admin', new Response('retention is at most 3650 days', { status: 400 }))
  fireEvent.change(await screen.findByLabelText('Delete audio after'), { target: { value: '365' } })
  expect((await screen.findByRole('alert')).textContent).toBe('retention is at most 3650 days')
})

test('members see the retention as text and no select', async () => {
  retention('member')
  expect((await screen.findByText(/deleted after/)).textContent).toMatch(/90 days/)
  expect(screen.queryByLabelText('Delete audio after')).toBeNull()
  expect(screen.queryByRole('combobox')).toBeNull()
})

const joined = { join_rule: 'manual', notetaker_name: 'Recapz notetaker', join_message: 'Hi from {name}. Type opt out to leave.' }

function joinSettings(role: string, get: () => Response, put?: Response) {
  const calls: { m: string; u: string; body?: string }[] = []
  vi.stubGlobal('fetch', vi.fn(async (u: string, init?: RequestInit) => {
    calls.push({ m: init?.method ?? 'GET', u, body: init?.body as string | undefined })
    if (u === '/api/settings') return init?.method === 'PUT' ? (put ?? new Response('{}')) : get()
    return new Response(JSON.stringify(
      u === '/api/me' ? { email: 'a@b.c', name: null, role }
        : u === '/api/usage' ? { speech_micro_usd: 0, llm_micro_usd: 0, total_micro_usd: 0, cap_micro_usd: 0, paused: false, meetings: [] }
          : { connected: false, needs_reconnect: false },
    ))
  }))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ToastProvider><MemoryRouter initialEntries={['/recording']}><Recording /></MemoryRouter></ToastProvider>
    </QueryClientProvider>,
  )
  return calls
}
const ok = () => new Response(JSON.stringify(joined))

test('admin edits the join settings: PUT carries all three, the preview follows, a toast says saved', async () => {
  const sent = { ...joined, join_rule: 'auto', notetaker_name: 'Scribe' }
  const calls = joinSettings('admin', ok, new Response(JSON.stringify(sent)))
  const name = await screen.findByLabelText('Notetaker name')
  expect(screen.getByLabelText('Message preview').textContent).toBe('Hi from Recapz notetaker. Type opt out to leave.')
  fireEvent.click(screen.getByLabelText(/Automatic/))
  fireEvent.change(name, { target: { value: 'Scribe' } })
  expect(screen.getByLabelText('Message preview').textContent).toMatch(/^Hi from Scribe\./)
  fireEvent.click(screen.getByRole('button', { name: 'Save' }))
  await waitFor(() => expect(calls.filter((c) => c.m === 'PUT').map((c) => c.body)).toEqual([JSON.stringify(sent)]))
  expect(await screen.findByText('Join settings saved')).toBeTruthy()
})

test('the preview adds the opt out wording when the message lacks it', async () => {
  joinSettings('admin', () => new Response(JSON.stringify({ ...joined, join_message: 'Recording.' })))
  await screen.findByLabelText('Join message')
  expect(screen.getByLabelText('Message preview').textContent).toBe("Recording. Type 'opt out' to make it leave and delete the recording.")
})

test('members see the join settings as text, with no inputs and no Save', async () => {
  joinSettings('member', ok)
  expect(await screen.findByText('Joining and the chat notice')).toBeTruthy()
  expect(await screen.findByText('Manual: The notetaker joins only meetings a member switched on.')).toBeTruthy()
  expect(screen.queryByLabelText('Notetaker name')).toBeNull()
  expect(screen.queryByRole('button', { name: 'Save' })).toBeNull()
})

test('a refused save shows the server message next to the field', async () => {
  joinSettings('admin', ok, new Response('notetaker name must be 1 to 60 characters', { status: 400 }))
  fireEvent.change(await screen.findByLabelText('Notetaker name'), { target: { value: '' } })
  fireEvent.click(screen.getByRole('button', { name: 'Save' }))
  const msg = await screen.findByRole('alert')
  expect(msg.textContent).toBe('notetaker name must be 1 to 60 characters')
  expect(msg.previousElementSibling?.id).toBe('notetaker-name')
})

test('join settings show loading, then an error when they cannot load', async () => {
  joinSettings('admin', () => new Response('x', { status: 500 }))
  expect(screen.getByText('Loading join settings...')).toBeTruthy()
  expect((await screen.findByRole('alert')).textContent).toBe('Could not load the join settings.')
  expect(screen.queryByLabelText('Notetaker name')).toBeNull()
})
