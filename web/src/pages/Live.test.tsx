import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import App from '../App'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const meeting = { id: 'm1', title: 'Client call', platform: 'meet', status: 'recording', language: 'vi', started_at: 1, ended_at: null, created_at: 1, error: null, speakers: [] }
const answerSse =
  'event: token\ndata: {"text":"Net 30."}\n\n' +
  'event: citations\ndata: [{"meeting_id":null,"document_id":"d1","title":"Rates.md","start_ms":null,"quote":"payment is net 30","answered":true},' +
  '{"meeting_id":null,"document_id":"d2","title":"Private.md","start_ms":null,"quote":"walrus escrow","answered":false}]\n\n' +
  'event: done\ndata: {}\n\n'

/** A live stream the test writes into, and the live-ask requests it saw. */
function mount(opts: { liveStatus?: number; me?: boolean; canEdit?: boolean } = {}) {
  const m = { ...meeting, can_edit: opts.canEdit ?? false }
  const controls: string[] = []
  const enc = new TextEncoder()
  let push: (s: string) => void = () => {}
  let close: () => void = () => {}
  const asks: unknown[] = []
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    if (url === '/api/me') return new Response(JSON.stringify({ email: 'ana@example.com', name: 'Ana', role: 'member' }))
    if (url === '/api/meetings') return new Response(JSON.stringify([m]))
    if (url === '/api/meetings/m1') return new Response(JSON.stringify(m))
    if (url.startsWith('/api/meetings/m1/live/') && init?.method === 'POST') { controls.push(url.split('/').pop()!); return new Response(null, { status: 204 }) }
    if (url === '/api/meetings/m1/live') {
      if (opts.liveStatus) return new Response('', { status: opts.liveStatus })
      return new Response(new ReadableStream({ start(c) { push = (s) => c.enqueue(enc.encode(s)); close = () => c.close() } }))
    }
    if (url === '/api/meetings/m1/live-ask') { asks.push(JSON.parse(init!.body as string)); return new Response(answerSse) }
    return new Response('', { status: 404 })
  }))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter initialEntries={['/meetings/m1/live']}><App /></MemoryRouter>
    </QueryClientProvider>,
  )
  const send = (name: string, data: object) => act(async () => { push(`event: ${name}\ndata: ${JSON.stringify(data)}\n\n`) })
  return { asks, controls, send, close: () => act(async () => { close() }) }
}

test('shows what is said as it arrives and the call ending', async () => {
  const { send, close } = mount()
  expect(await screen.findByText('Waiting for the first words…')).toBeTruthy()
  await send('partial', { text: 'good mor' })
  expect(await screen.findByText('good mor')).toBeTruthy()
  await send('final', { at_ms: 65000, text: 'good morning' })
  expect(await screen.findByText('good morning')).toBeTruthy()
  expect(screen.getByText('1:05')).toBeTruthy()
  expect(screen.queryByText('good mor')).toBeNull()
  await send('end', {})
  await close()
  expect(await screen.findByText('Call ended')).toBeTruthy()
  expect((screen.getByRole('button', { name: /Ask now/ }) as HTMLButtonElement).disabled).toBe(true)
})

test('says the live transcript is unavailable, and the same when the connection drops', async () => {
  const a = mount()
  await a.send('unavailable', {})
  expect((await screen.findByRole('status')).textContent).toContain('Live transcript unavailable')
  cleanup()
  const b = mount()
  await screen.findByText('Waiting for the first words…')
  await b.close()
  expect((await screen.findByRole('status')).textContent).toContain('Live transcript unavailable')
})

test('a meeting that is not live says so', async () => {
  mount({ liveStatus: 404 })
  expect(await screen.findByText('This meeting is not being recorded right now.')).toBeTruthy()
})

test('Ask now sends the last 60 s, then a clicked line and the typed words, and shows answer and passages only', async () => {
  const { asks, send } = mount()
  await send('final', { at_ms: 5000, text: 'what are the terms' })
  fireEvent.click(await screen.findByRole('button', { name: /Ask now/ }))
  expect(await screen.findByText('Net 30.')).toBeTruthy()
  expect(asks).toEqual([{}])
  expect(screen.getByText('Last 60 s')).toBeTruthy()
  expect(screen.getByText('Rates.md')).toBeTruthy()
  expect(screen.getByText('Passages only')).toBeTruthy()
  expect(screen.getByText('walrus escrow')).toBeTruthy()

  fireEvent.click(screen.getByRole('button', { name: /what are the terms/ }))
  expect(screen.getByText('Asking about the line at 0:05')).toBeTruthy()
  fireEvent.change(screen.getByLabelText('Extra words'), { target: { value: ' and the deposit ' } })
  fireEvent.click(screen.getByRole('button', { name: /Ask now/ }))
  await waitFor(() => expect(asks).toHaveLength(2))
  expect(asks[1]).toEqual({ at_ms: 5000, text: 'and the deposit' })
  expect(screen.getAllByText('Net 30.')).toHaveLength(2)

  fireEvent.click(screen.getByRole('button', { name: 'use last 60 s' }))
  expect(screen.getByText('Asking about the last 60 s')).toBeTruthy()
})

test('the A key asks, except while typing', async () => {
  const { asks } = mount()
  await screen.findByRole('button', { name: /Ask now/ })
  fireEvent.keyDown(screen.getByLabelText('Extra words'), { key: 'a' })
  expect(asks).toHaveLength(0)
  fireEvent.keyDown(document.body, { key: 'a' })
  await waitFor(() => expect(asks).toHaveLength(1))
})

test('a refused ask shows why on its card', async () => {
  mount()
  vi.stubGlobal('fetch', vi.fn(async (url: string) => (url.endsWith('/live-ask') ? new Response('', { status: 429 }) : new Response(new ReadableStream()))))
  fireEvent.click(await screen.findByRole('button', { name: /Ask now/ }))
  expect((await screen.findByRole('alert')).textContent).toBe('Too many questions. Try again in a minute.')
})

test('the rail offers the live meeting while a call is recorded', async () => {
  mount()
  const link = await screen.findByRole('link', { name: 'Live meeting' })
  expect(link.getAttribute('href')).toBe('/meetings/m1/live')
  expect(link.getAttribute('aria-current')).toBe('page')
})

test('with two calls recording the rail lists each by title', async () => {
  const other = { ...meeting, id: 'm2', title: 'Board call' }
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    if (url === '/api/me') return new Response(JSON.stringify({ email: 'ana@example.com', name: 'Ana', role: 'member' }))
    if (url === '/api/meetings') return new Response(JSON.stringify([meeting, other, { ...meeting, id: 'm3', title: 'Done', status: 'ready' }]))
    return new Response('', { status: 404 })
  }))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter initialEntries={['/']}><App /></MemoryRouter>
    </QueryClientProvider>,
  )
  const a = await screen.findByRole('link', { name: 'Live: Client call' })
  const b = screen.getByRole('link', { name: 'Live: Board call' })
  expect([a.getAttribute('href'), b.getAttribute('href')]).toEqual(['/meetings/m1/live', '/meetings/m2/live'])
  expect(within(screen.getByRole('navigation', { name: 'Screens' })).queryByRole('link', { name: /Done/ })).toBeNull()
})

test('shows platform, join time and a timer that stops when the call ends', async () => {
  const start = 1_700_000_000_000
  vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'], now: start + 5000 })
  try {
    meeting.started_at = start
    const { send, close } = mount()
    const bar = await screen.findByLabelText('Elapsed')
    expect(bar.textContent).toBe('0:05')
    expect(screen.getByText('meet')).toBeTruthy()
    expect(screen.getByText(/Joined/).textContent).toContain(new Date(start).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }))
    await act(async () => { vi.setSystemTime(start + 65000); vi.advanceTimersByTime(1000) })
    expect(screen.getByLabelText('Elapsed').textContent).toBe('1:06')
    await send('end', {})
    await close()
    expect(await screen.findByText('Call ended')).toBeTruthy()
    await act(async () => { vi.setSystemTime(start + 600000); vi.advanceTimersByTime(5000) })
    expect(screen.getByLabelText('Elapsed').textContent).toBe('1:06')
  } finally { meeting.started_at = 1; vi.useRealTimers() }
})

test('the owner or an admin pauses, resumes and stops the recording; others get no controls', async () => {
  const { controls, send } = mount({ canEdit: true })
  fireEvent.click(await screen.findByRole('button', { name: 'Pause recording' }))
  await waitFor(() => expect(controls).toEqual(['pause']))
  await send('paused', { at_ms: 61000 })
  expect(await screen.findByText('Paused')).toBeTruthy()
  expect(screen.getByText('Recording paused')).toBeTruthy()
  expect(screen.getByText('1:01')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Resume recording' }))
  await waitFor(() => expect(controls).toEqual(['pause', 'resume']))
  await send('resumed', { at_ms: 90000 })
  expect(await screen.findByText('Recording resumed')).toBeTruthy()
  expect(screen.queryByText('Paused')).toBeNull()
  // Stopping asks first: Cancel sends nothing, the confirm sends one stop.
  fireEvent.click(screen.getByRole('button', { name: 'Stop and leave' }))
  fireEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Cancel' }))
  expect(controls).toEqual(['pause', 'resume'])
  fireEvent.click(screen.getByRole('button', { name: 'Stop and leave' }))
  fireEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Stop and leave' }))
  await waitFor(() => expect(controls).toEqual(['pause', 'resume', 'stop']))
})

test('a member who cannot edit the meeting sees no recording controls', async () => {
  mount()
  await screen.findByText('Waiting for the first words…')
  expect(screen.queryByRole('button', { name: 'Pause recording' })).toBeNull()
  expect(screen.queryByRole('button', { name: 'Stop and leave' })).toBeNull()
})

test('shows who is speaking: names on lines, the ribbon, speaking now and talk time', async () => {
  const { send } = mount()
  await send('final', { at_ms: 4000, text: 'hello all', speaker: 0, label: 'Speaker 1', ms: 3000 })
  await send('final', { at_ms: 9000, text: 'hi there', speaker: 1, label: 'Speaker 2', ms: 1000 })
  // Speaker 1 is later named: the name replaces the number everywhere, and talk time adds up per speaker.
  await send('final', { at_ms: 12000, text: 'and so', speaker: 0, label: 'Ann', ms: 2000 })
  expect(await screen.findByText(/Speaking now/)).toBeTruthy()
  expect(screen.getByText(/Speaking now/).textContent).toBe('Speaking now: Ann')
  const talkTime = within(screen.getByRole('list', { name: 'Talk time' })).getAllByRole('listitem').map((li) => li.textContent)
  expect(talkTime).toEqual(['Ann0:05', 'Speaker 20:01'])
  expect(screen.getByText('hello all').parentElement!.textContent).toBe('0:04Ann:hello all')
  const strip = document.querySelector('[data-variant="live"]')!
  expect([...strip.children].map((c) => [c.getAttribute('data-speaker'), (c as HTMLElement).style.flex])).toEqual([['0', '3000 1 0%'], ['1', '1000 1 0%'], ['0', '2000 1 0%']])
})

test('lines without a speaker show no speaker block', async () => {
  const { send } = mount()
  await send('final', { at_ms: 4000, text: 'plain line' })
  await screen.findByText('plain line')
  expect(screen.queryByLabelText('Speakers')).toBeNull()
})
