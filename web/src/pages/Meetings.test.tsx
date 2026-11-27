import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { ToastProvider } from '../ui/Toast'
import { Meetings } from './Meetings'

afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers() })

const show = () =>
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ToastProvider><MemoryRouter><Meetings /></MemoryRouter></ToastProvider>
    </QueryClientProvider>,
  )

test('lists meetings from the API', async () => {
  const rows = [{ id: '1', title: 'Weekly sync', platform: 'upload', status: 'uploaded', started_at: null, created_at: 1 }]
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(rows))))
  show()
  expect(await screen.findByText('Weekly sync')).toBeTruthy()
})

test('shows a day heading, the status badge and the month count for each meeting', async () => {
  const now = new Date(2026, 10, 12, 12).getTime()
  vi.useFakeTimers({ toFake: ['Date'], now })
  const rows = [
    { id: '1', title: 'Weekly sync', platform: 'upload', status: 'ready', started_at: now, created_at: now },
    { id: '2', title: 'Broken one', platform: 'meet', status: 'failed', started_at: now - 1000, created_at: now },
    { id: '3', title: 'Working one', platform: 'meet', status: 'transcribing', started_at: now - 2000, created_at: now },
    { id: '4', title: 'Odd one', platform: 'meet', status: 'toString', started_at: now - 3000, created_at: now },
  ]
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(rows))))
  show()
  expect(await screen.findByRole('heading', { name: 'Today' })).toBeTruthy()
  expect(screen.getAllByRole('heading', { level: 2 }).map((h) => h.textContent)).toEqual(['Today', 'Next up today'])
  expect(screen.getByRole('link', { name: 'Upload' }).getAttribute('href')).toBe('/upload')
  expect(screen.getByText('Ready').className).toContain('text-good')
  expect(screen.getByText('Failed').className).toContain('text-bad')
  expect(screen.getByText('Transcribing').className).toContain('text-signal-ink')
  expect(screen.getByText('toString').className).toContain('text-ink-2')
  expect(screen.getByText(/4 meetings this month/)).toBeTruthy()
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
  expect(screen.getByRole('status').textContent).toBe('Meeting deleted')
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

const sendNotetaker = async (f: ReturnType<typeof vi.fn>, link: string, title = '') => {
  vi.stubGlobal('fetch', f)
  show()
  fireEvent.click(await screen.findByRole('button', { name: 'Send notetaker' }))
  fireEvent.change(screen.getByLabelText('Meeting link'), { target: { value: link } })
  if (title) fireEvent.change(screen.getByLabelText('Title (optional)'), { target: { value: title } })
  fireEvent.click(within(screen.getByRole('dialog', { name: 'Send notetaker' })).getByRole('button', { name: 'Send notetaker' }))
}
const joins = (f: ReturnType<typeof vi.fn>) => f.mock.calls.filter(([u]) => u === '/api/meetings/join')

test('send notetaker posts the link and title, shows loading, then refreshes the list', async () => {
  let rows: unknown[] = []
  let release = () => {}
  const f = vi.fn(async (u: string) => {
    if (u === '/api/meetings/join') {
      await new Promise<void>((r) => { release = r })
      rows = [{ id: '9', title: 'Design review', platform: 'zoom', status: 'joining', started_at: null, created_at: 1 }]
      return new Response('{"id":"9"}', { status: 201 })
    }
    return new Response(JSON.stringify(rows))
  })
  await sendNotetaker(f, ' https://us02web.zoom.us/j/84012345678 ', ' Design review ')
  expect(await screen.findByRole('button', { name: 'Sending…' })).toHaveProperty('disabled', true)
  release()
  expect(await screen.findByText('Design review')).toBeTruthy()
  expect(joins(f)).toHaveLength(1)
  const init = joins(f)[0][1] as RequestInit
  expect(init.method).toBe('POST')
  expect(JSON.parse(init.body as string)).toEqual({ url: 'https://us02web.zoom.us/j/84012345678', title: 'Design review' })
  expect(screen.queryByRole('dialog', { name: 'Send notetaker' })).toBeNull()
  expect(screen.getByRole('status').textContent).toBe('Notetaker is on its way')
})

test('send notetaker without a title omits it', async () => {
  const f = vi.fn(async (u: string) => u === '/api/meetings/join' ? new Response('{"id":"9"}', { status: 201 }) : new Response('[]'))
  await sendNotetaker(f, 'https://meet.google.com/abc-defg-hij')
  await waitFor(() => expect(joins(f)).toHaveLength(1))
  expect(JSON.parse((joins(f)[0][1] as RequestInit).body as string)).toEqual({ url: 'https://meet.google.com/abc-defg-hij' })
})

test('send notetaker shows the server error and keeps the form open', async () => {
  const f = vi.fn(async (u: string) => u === '/api/meetings/join' ? new Response('not a Meet or Zoom link', { status: 400 }) : new Response('[]'))
  await sendNotetaker(f, 'https://example.com/x')
  expect((await screen.findByRole('alert')).textContent).toBe('not a Meet or Zoom link')
  expect((screen.getByLabelText('Meeting link') as HTMLInputElement).value).toBe('https://example.com/x')
})

test('an empty list says so and links to Upload; a failed load shows an alert', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('[]')))
  show()
  expect((await screen.findByText(/No meetings yet/)).textContent).toContain('Upload a recording')
  cleanup()
  vi.stubGlobal('fetch', vi.fn(async () => new Response('x', { status: 500 })))
  show()
  expect((await screen.findByRole('alert', {}, { timeout: 5000 })).textContent).toBe('Could not load meetings.')
}, 8000)

test('recording and joining meetings get their stage badge; Send notetaker opens a dialog', async () => {
  const rows = [{ id: '1', title: 'Live one', platform: 'meet', status: 'recording', started_at: 5, created_at: 5 }]
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(rows))))
  show()
  // The row keeps its badge next to the card's.
  const row = (await screen.findByRole('link', { name: 'Live one' })).closest('li')!
  expect(within(row).getByText('Recording').className).toContain('text-signal-ink')
  fireEvent.click(screen.getByRole('button', { name: 'Send notetaker' }))
  expect(screen.getByRole('dialog', { name: 'Send notetaker' })).toBeTruthy()
  fireEvent.keyDown(document.body, { key: 'Escape' })
  expect(screen.queryByRole('dialog')).toBeNull()
})

test('Tab wraps inside the Send notetaker dialog', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('[]')))
  show()
  fireEvent.click(await screen.findByRole('button', { name: 'Send notetaker' }))
  const dialog = screen.getByRole('dialog', { name: 'Send notetaker' })
  const send = within(dialog).getByRole('button', { name: 'Send notetaker' })
  send.focus()
  fireEvent.keyDown(send, { key: 'Tab' })
  expect(document.activeElement).toBe(screen.getByLabelText('Meeting link'))
  fireEvent.keyDown(document.activeElement!, { key: 'Tab', shiftKey: true })
  expect(document.activeElement).toBe(send)
})

test('shows duration, action items, ribbon and faces from the list data, and hides what is missing', async () => {
  const now = new Date(2026, 10, 12, 12).getTime()
  vi.useFakeTimers({ toFake: ['Date'], now })
  const rows = [
    { id: '1', title: 'Full row', platform: 'meet', status: 'ready', started_at: now, created_at: now, duration_ms: 48 * 60_000, action_item_count: 3,
      speakers: [{ id: 'a', name: 'Ada' }, { id: 'b', name: 'Ben' }] },
    { id: '2', title: 'Bare row', platform: 'upload', status: 'uploaded', started_at: now - 1000, created_at: now, duration_ms: null, action_item_count: 0, speakers: [] },
    { id: '3', title: 'One item', platform: 'meet', status: 'ready', started_at: now - 2000, created_at: now, duration_ms: null, action_item_count: 1, speakers: [] },
  ]
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(rows))))
  show()
  const full = (await screen.findByText('Full row')).closest('li')!
  const bare = screen.getByText('Bare row').closest('li')!
  expect(within(full).getByText('48 min')).toBeTruthy()
  expect(within(full).getByText('3 action items')).toBeTruthy()
  expect(within(screen.getByText('One item').closest('li')!).getByText('1 action item')).toBeTruthy()
  expect([...full.querySelectorAll('[data-variant="list"] > span')].map((s) => s.getAttribute('data-speaker'))).toEqual(['0', '1'])
  expect(within(full).getAllByRole('img').map((i) => i.getAttribute('aria-label'))).toEqual(['Ada', 'Ben'])
  expect(bare.textContent).not.toMatch(/min|action item/)
  expect(bare.querySelector('[data-variant]')).toBeNull()
  expect(within(bare).queryAllByRole('img')).toEqual([])
  expect(screen.getByText('3 meetings this month · 48 min recorded')).toBeTruthy()
  expect(document.body.textContent).not.toMatch(/stays on your server/)
})

test('the month line names no speech provider', async () => {
  const rows = [{ id: '1', title: 'Weekly sync', platform: 'meet', status: 'ready', started_at: Date.now(), created_at: 1 }]
  vi.stubGlobal('fetch', vi.fn(async (url: string) => new Response(JSON.stringify(url === '/api/me' ? { email: 'a@example.com', name: null, role: 'member' } : rows))))
  show()
  expect(await screen.findByText(/^1 meetings? this month/)).toBeTruthy()
  expect(document.body.textContent).not.toMatch(/processed by|Deepgram/)
})

test('a call being recorded gets a card with its elapsed time that keeps counting and a link to its live view', async () => {
  const now = new Date(2026, 10, 12, 12).getTime()
  vi.useFakeTimers({ toFake: ['Date'], now })
  const rows = [
    { id: 'r1', title: 'Client call', platform: 'meet', status: 'recording', started_at: now - 125_000, created_at: now },
    { id: 'r2', title: 'Board call', platform: 'meet', status: 'recording', started_at: now - 3_725_000, created_at: now },
    { id: 'd1', title: 'Finished call', platform: 'meet', status: 'ready', started_at: now - 9_000_000, created_at: now },
  ]
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(rows))))
  show()
  const cards = await screen.findAllByTestId('live-card')
  expect(cards).toHaveLength(2)
  expect(within(cards[0]).getByText('Client call')).toBeTruthy()
  expect(within(cards[0]).getByText('2:05')).toBeTruthy()
  expect(within(cards[1]).getByText('1:02:05')).toBeTruthy()
  expect(within(cards[0]).getByRole('link', { name: 'Open live view' }).getAttribute('href')).toBe('/meetings/r1/live')
  expect(within(cards[1]).getByRole('link', { name: 'Open live view' }).getAttribute('href')).toBe('/meetings/r2/live')
  vi.setSystemTime(now + 5_000)
  await waitFor(() => expect(within(cards[0]).getByText('2:10')).toBeTruthy(), { timeout: 2500 })
})

test('a recording call shows a speaker ribbon once someone has spoken', async () => {
  const rows = [{ id: 'r1', title: 'Client call', platform: 'meet', status: 'recording', started_at: 1, created_at: 1 }]
  const sse = 'event: final\ndata: {"at_ms":1000,"text":"hi","speaker":0,"label":"Speaker 1","ms":3000}\n\n' +
    'event: final\ndata: {"at_ms":2000,"text":"yo","speaker":1,"label":"Speaker 2","ms":1000}\n\n'
  vi.stubGlobal('fetch', vi.fn(async (url: string) => new Response(url === '/api/meetings/r1/live' ? sse : JSON.stringify(rows))))
  show()
  const card = await screen.findByTestId('live-card')
  await waitFor(() => expect(card.querySelectorAll('[data-variant="card"] > span')).toHaveLength(2))
  expect([...card.querySelectorAll('[data-variant="card"] > span')].map((c) => c.getAttribute('data-speaker'))).toEqual(['0', '1'])
})

test('no card without a recording call', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify([{ id: '1', title: 'Weekly sync', platform: 'meet', status: 'ready', started_at: 1, created_at: 1 }]))))
  show()
  await screen.findByText('Weekly sync')
  expect(screen.queryByTestId('live-card')).toBeNull()
})
