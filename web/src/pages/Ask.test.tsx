import { cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { parseEvents } from '../api'
import { Ask } from './Ask'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

/** Answers the history load with `history` and hands every other request to the returned mock. */
function stubFetch(handler: () => Response, history: unknown[] = []) {
  const calls = vi.fn(async (_url: string, _init?: RequestInit) => handler())
  vi.stubGlobal('fetch', async (url: string, init?: RequestInit) =>
    url === '/api/ask/history' && !init?.method ? Response.json(history) : calls(url, init))
  return calls
}

const sse =
  'event: token\ndata: {"text":"Ships Friday ["}\n\nevent: token\ndata: {"text":"1]. Not [9]."}\n\n' +
  'event: citations\ndata: [{"meeting_id":"m1","document_id":null,"title":"Weekly sync","start_ms":65000,"quote":"pricing page ships Friday","answered":true,"n":1,"speaker":"Bo","duration_ms":200000,"ribbon":[{"speaker":0,"weight":30},{"speaker":1,"weight":30}]}]\n\n' +
  'event: done\ndata: {}\n\n'

test('shows the streamed answer and a citation card that opens the moment', async () => {
  const fetchMock = stubFetch(() => new Response(sse))
  render(<MemoryRouter><Ask /></MemoryRouter>)
  fireEvent.change(screen.getByLabelText('Question'), { target: { value: ' when does pricing ship? ' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
  // The marker is a superscript link; [9] has no card, so it is dropped, never shown raw.
  expect(await screen.findByText((_, el) => el?.tagName === 'P' && el.textContent === 'Ships Friday 1. Not .')).toBeTruthy()
  expect(screen.getByRole('link', { name: '1' }).getAttribute('href')).toBe('#cite-0-0')
  expect(screen.getByTestId('pin').style.left).toBe('32.5%')
  expect(screen.getByRole('img', { name: 'Speaker ribbon, moment at 1:05' }).querySelectorAll('[data-speaker]')).toHaveLength(2)
  expect(screen.getByText('Bo')).toBeTruthy()
  const card = (await screen.findByText('pricing page ships Friday')).closest('a')!
  expect(card.getAttribute('href')).toBe('/meetings/m1?t=65000')
  expect(screen.getByText('1:05')).toBeTruthy()
  const call = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
  expect(call[0]).toBe('/api/ask')
  expect(JSON.parse(call[1].body as string)).toEqual({ question: 'when does pricing ship?' })
})

async function askWith(res: () => Response) {
  stubFetch(res)
  render(<MemoryRouter><Ask /></MemoryRouter>)
  fireEvent.change(screen.getByLabelText('Question'), { target: { value: 'x' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
}

test.each([
  [401, 'Sign in to ask.'],
  [429, 'Too many questions. Try again in a minute.'],
  [503, 'Ask needs a language model. Ask an admin to configure one.'],
  [500, 'Could not get an answer.'],
])('status %i shows its message', async (status, text) => {
  await askWith(() => new Response('', { status }))
  expect((await screen.findByRole('alert')).textContent).toBe(text)
})

test('an error event ends the answer with an error', async () => {
  await askWith(() => new Response('event: token\ndata: {"text":"Par"}\n\nevent: error\ndata: {"message":"m"}\n\n'))
  expect((await screen.findByRole('alert')).textContent).toBe('Could not get an answer.')
})

test('a stream error keeps the partial answer above the alert', async () => {
  await askWith(() => new Response('event: token\ndata: {"text":"Par"}\n\nevent: error\ndata: {"message":"m"}\n\n'))
  expect((await screen.findByRole('alert')).textContent).toBe('Could not get an answer.')
  expect(screen.getByText('Par')).toBeTruthy()
})

test('a failed request puts the typed question back', async () => {
  await askWith(() => new Response('', { status: 429 }))
  await screen.findByRole('alert')
  expect((screen.getByLabelText('Question') as HTMLInputElement).value).toBe('x')
})

test('only three markers are shown for four citations', async () => {
  const c = [1, 2, 3, 4].map((n) => ({ meeting_id: `m${n}`, document_id: null, answered: true, n, title: `T${n}`, start_ms: n * 1000, quote: `q${n}` }))
  await askWith(() => new Response(`event: token\ndata: {"text":"A [1][2][3][4]"}\n\nevent: citations\ndata: ${JSON.stringify(c)}\n\nevent: done\ndata: {}\n\n`))
  await screen.findByText('q3')
  expect(screen.queryByText('q4')).toBeNull()
  expect(screen.queryByRole('link', { name: '4' })).toBeNull()
  expect(screen.getByRole('link', { name: '3' })).toBeTruthy()
})

test('while streaming a marker shows as a plain number and an unfinished one waits', async () => {
  let push!: (t: string) => void
  const body = new ReadableStream<Uint8Array>({ start(c) { push = (t) => c.enqueue(new TextEncoder().encode(t)) } })
  stubFetch(() => new Response(body))
  render(<MemoryRouter><Ask /></MemoryRouter>)
  fireEvent.change(screen.getByLabelText('Question'), { target: { value: 'x' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
  push('event: token\ndata: {"text":"May [2] and ["}\n\n')
  const p = await screen.findByText((_, el) => el?.tagName === 'P' && el.textContent === 'May 2 and ')
  expect(p.querySelector('sup')!.textContent).toBe('2')
  expect(p.querySelector('a')).toBeNull()
})

test('a document citation names the document and links to its page, not a meeting', async () => {
  const c = [{ meeting_id: null, document_id: 'd1', answered: false, title: 'notes.md', start_ms: null, quote: 'walrus clause' }]
  await askWith(() => new Response(`event: token\ndata: {"text":"A"}\n\nevent: citations\ndata: ${JSON.stringify(c)}\n\nevent: done\ndata: {}\n\n`))
  expect(await screen.findByText('notes.md')).toBeTruthy()
  expect(screen.getByText('Document, not sent to the model')).toBeTruthy()
  expect(screen.getByRole('link', { name: /notes\.md/ }).getAttribute('href')).toBe('/documents/d1')
})

test('no citations says so', async () => {
  await askWith(() => new Response('event: token\ndata: {"text":"Nothing."}\n\nevent: citations\ndata: []\n\nevent: done\ndata: {}\n\n'))
  expect(await screen.findByText('No matching moments found.')).toBeTruthy()
})

test('answer markers link to cards, and a chip asks a second question below a divider', async () => {
  const fetchMock = stubFetch(() => new Response(sse))
  const { container } = render(<MemoryRouter><Ask /></MemoryRouter>)
  fireEvent.click(screen.getByRole('button', { name: 'What are the open action items?' }))
  const marker = await screen.findByRole('link', { name: '1' })
  expect(marker.getAttribute('href')).toBe('#cite-0-0')
  expect(container.querySelector('#cite-0-0')!.getAttribute('href')).toBe('/meetings/m1?t=65000')
  await screen.findByText('1:05')
  fireEvent.change(screen.getByLabelText('Question'), { target: { value: 'again' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
  await vi.waitFor(() => expect(screen.getAllByText('1:05')).toHaveLength(2))
  expect(JSON.parse((fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1].body as string))
    .toEqual({ question: 'What are the open action items?' })
  expect(screen.getByText('again')).toBeTruthy()
})

test('parseEvents keeps an unfinished event for the next read', () => {
  const r = parseEvents('event: token\ndata: {"text":"a"}\n\nevent: tok')
  expect(r.events).toEqual([{ name: 'token', data: { text: 'a' } }])
  expect(r.rest).toBe('event: tok')
})

test('filters are sent with the question only when set, and Clear removes them', async () => {
  const fetchMock = stubFetch(() => new Response(sse))
  render(<MemoryRouter><Ask /></MemoryRouter>)
  expect(screen.queryByRole('button', { name: 'Clear filters' })).toBeNull()
  fireEvent.change(screen.getByLabelText('From'), { target: { value: '2026-11-01' } })
  fireEvent.change(screen.getByLabelText('Person'), { target: { value: 'Linh' } })
  fireEvent.change(screen.getByLabelText('Question'), { target: { value: 'what did she say?' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
  await screen.findByText('pricing page ships Friday')
  const body = (i: number) => JSON.parse((fetchMock.mock.calls[i] as unknown as [string, RequestInit])[1].body as string)
  expect(body(0)).toEqual({ question: 'what did she say?', from: '2026-11-01', speaker: 'Linh' })
  fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }))
  expect((screen.getByLabelText('Person') as HTMLInputElement).value).toBe('')
  fireEvent.change(screen.getByLabelText('Question'), { target: { value: 'again' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
  await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2))
  expect(body(1)).toEqual({ question: 'again' })
})

const saved = (id: string, q: string) => ({
  id, question: q, answer: `Answer ${id} [1]`, created_at: 1,
  citations: [{ meeting_id: 'm1', document_id: null, title: 'Weekly sync', start_ms: 65000, quote: `quote ${id}`, answered: true, n: 1 }],
})

test('saved turns show again on open, newest first, with their citation cards', async () => {
  stubFetch(() => new Response(''), [saved('t1', 'first question'), saved('t2', 'second question')])
  render(<MemoryRouter><Ask /></MemoryRouter>)
  expect(await screen.findByText('first question')).toBeTruthy()
  const qs = screen.getAllByText(/question$/).map((e) => e.textContent)
  expect(qs).toEqual(['second question', 'first question'])
  expect(screen.getByText('quote t1').closest('a')!.getAttribute('href')).toBe('/meetings/m1?t=65000')
  expect(screen.getByText('quote t2')).toBeTruthy()
})

test('a new answer can be deleted without a reload, using the id from the done event', async () => {
  const calls = stubFetch(() => new Response('event: token\ndata: {"text":"Hi"}\n\nevent: citations\ndata: []\n\nevent: done\ndata: {"id":"new-1"}\n\n'))
  render(<MemoryRouter><Ask /></MemoryRouter>)
  fireEvent.change(screen.getByLabelText('Question'), { target: { value: 'fresh one' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
  fireEvent.click(await screen.findByRole('button', { name: 'Delete this question' }))
  fireEvent.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Delete' }))
  await vi.waitFor(() => expect(screen.queryByText('fresh one')).toBeNull())
  const del = calls.mock.calls[1] as unknown as [string, RequestInit]
  expect([del[0], del[1].method]).toEqual(['/api/ask/history/new-1', 'DELETE'])
})

test('deleting one turn removes only that turn', async () => {
  const calls = stubFetch(() => new Response(null, { status: 204 }), [saved('t1', 'first question'), saved('t2', 'second question')])
  render(<MemoryRouter><Ask /></MemoryRouter>)
  await screen.findByText('first question')
  fireEvent.click(screen.getAllByRole('button', { name: 'Delete this question' })[1])
  fireEvent.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Delete' }))
  await vi.waitFor(() => expect(screen.queryByText('first question')).toBeNull())
  expect(screen.getByText('second question')).toBeTruthy()
  expect(calls.mock.calls.map((c) => [c[0], c[1]?.method])).toEqual([['/api/ask/history/t1', 'DELETE']])
})

test('Clear history asks first, cancel keeps everything, confirm deletes all', async () => {
  const calls = stubFetch(() => new Response(null, { status: 204 }), [saved('t1', 'first question'), saved('t2', 'second question')])
  render(<MemoryRouter><Ask /></MemoryRouter>)
  await screen.findByText('first question')
  fireEvent.click(screen.getByRole('button', { name: 'Clear history' }))
  fireEvent.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Cancel' }))
  expect(screen.getByText('second question')).toBeTruthy()
  expect(calls).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'Clear history' }))
  fireEvent.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Clear history' }))
  await vi.waitFor(() => expect(screen.queryByText('first question')).toBeNull())
  expect(screen.queryByText('second question')).toBeNull()
  expect(calls.mock.calls.map((c) => [c[0], c[1]?.method])).toEqual([['/api/ask/history', 'DELETE']])
})

test('a failed delete keeps the turn and says so', async () => {
  stubFetch(() => new Response('', { status: 500 }), [saved('t1', 'first question')])
  render(<MemoryRouter><Ask /></MemoryRouter>)
  await screen.findByText('first question')
  fireEvent.click(screen.getByRole('button', { name: 'Delete this question' }))
  fireEvent.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Delete' }))
  expect(await screen.findByText('Could not delete. Try again.')).toBeTruthy()
  expect(screen.getByText('first question')).toBeTruthy()
})

test('with no saved turns there is no Clear history button', async () => {
  stubFetch(() => new Response(''))
  render(<MemoryRouter><Ask /></MemoryRouter>)
  await Promise.resolve()
  expect(screen.queryByRole('button', { name: 'Clear history' })).toBeNull()
})

test('the question box is above the history, and a new question lands on top of it', async () => {
  stubFetch(() => new Response('event: token\ndata: {"text":"Hi"}\n\nevent: citations\ndata: []\n\nevent: done\ndata: {"id":"n1"}\n\n'),
    [saved('t1', 'first question'), saved('t2', 'second question')])
  render(<MemoryRouter><Ask /></MemoryRouter>)
  await screen.findByText('first question')
  const before = (a: Node, b: Node) => !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING)
  const box = screen.getByLabelText('Question')
  expect(before(box, screen.getByText('second question'))).toBe(true)
  expect(before(screen.getByRole('button', { name: 'What are the open action items?' }), screen.getByText('second question'))).toBe(true)
  fireEvent.change(box, { target: { value: 'newest one' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
  await screen.findByText('Hi')
  expect(screen.getAllByText(/(question|one)$/).map((e) => e.textContent)).toEqual(['newest one', 'second question', 'first question'])
})
