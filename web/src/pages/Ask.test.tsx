import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { parseEvents } from '../api'
import { Ask } from './Ask'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const sse =
  'event: token\ndata: {"text":"Ships Friday ["}\n\nevent: token\ndata: {"text":"1]. Not [9]."}\n\n' +
  'event: citations\ndata: [{"meeting_id":"m1","document_id":null,"title":"Weekly sync","start_ms":65000,"quote":"pricing page ships Friday","answered":true,"n":1,"speaker":"Bo","duration_ms":200000,"ribbon":[{"speaker":0,"weight":30},{"speaker":1,"weight":30}]}]\n\n' +
  'event: done\ndata: {}\n\n'

test('shows the streamed answer and a citation card that opens the moment', async () => {
  const fetchMock = vi.fn(async () => new Response(sse))
  vi.stubGlobal('fetch', fetchMock)
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
  vi.stubGlobal('fetch', vi.fn(async () => res()))
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
  vi.stubGlobal('fetch', vi.fn(async () => new Response(body)))
  render(<MemoryRouter><Ask /></MemoryRouter>)
  fireEvent.change(screen.getByLabelText('Question'), { target: { value: 'x' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
  push('event: token\ndata: {"text":"May [2] and ["}\n\n')
  const p = await screen.findByText((_, el) => el?.tagName === 'P' && el.textContent === 'May 2 and ')
  expect(p.querySelector('sup')!.textContent).toBe('2')
  expect(p.querySelector('a')).toBeNull()
})

test('a document citation names the document and is not a meeting link', async () => {
  const c = [{ meeting_id: null, document_id: 'd1', answered: false, title: 'notes.md', start_ms: null, quote: 'walrus clause' }]
  await askWith(() => new Response(`event: token\ndata: {"text":"A"}\n\nevent: citations\ndata: ${JSON.stringify(c)}\n\nevent: done\ndata: {}\n\n`))
  expect(await screen.findByText('notes.md')).toBeTruthy()
  expect(screen.getByText('Document, not sent to the model')).toBeTruthy()
  expect(screen.queryByRole('link', { name: /notes\.md/ })).toBeNull()
})

test('no citations says so', async () => {
  await askWith(() => new Response('event: token\ndata: {"text":"Nothing."}\n\nevent: citations\ndata: []\n\nevent: done\ndata: {}\n\n'))
  expect(await screen.findByText('No matching moments found.')).toBeTruthy()
})

test('answer markers link to cards, and a chip asks a second question below a divider', async () => {
  const fetchMock = vi.fn(async () => new Response(sse))
  vi.stubGlobal('fetch', fetchMock)
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
  const fetchMock = vi.fn(async () => new Response(sse))
  vi.stubGlobal('fetch', fetchMock)
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
