import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { parseEvents } from '../api'
import { Ask } from './Ask'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const sse =
  'event: token\ndata: {"text":"Ships "}\n\nevent: token\ndata: {"text":"Friday."}\n\n' +
  'event: citations\ndata: [{"meeting_id":"m1","title":"Weekly sync","start_ms":65000,"quote":"pricing page ships Friday"}]\n\n' +
  'event: done\ndata: {}\n\n'

test('shows the streamed answer and a citation card that opens the moment', async () => {
  const fetchMock = vi.fn(async () => new Response(sse))
  vi.stubGlobal('fetch', fetchMock)
  render(<MemoryRouter><Ask /></MemoryRouter>)
  fireEvent.change(screen.getByLabelText('Question'), { target: { value: ' when does pricing ship? ' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
  expect(await screen.findByText('Ships Friday.')).toBeTruthy()
  const card = (await screen.findByText('pricing page ships Friday')).closest('a')!
  expect(card.getAttribute('href')).toBe('/meetings/m1?t=65000')
  expect(screen.getByText('1:05')).toBeTruthy()
  const call = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
  expect(call[0]).toBe('/api/ask')
  expect(JSON.parse(call[1].body as string)).toEqual({ question: 'when does pricing ship?' })
})

test('tells the user when asking fails', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 503 })))
  render(<MemoryRouter><Ask /></MemoryRouter>)
  fireEvent.change(screen.getByLabelText('Question'), { target: { value: 'x' } })
  fireEvent.click(screen.getByRole('button', { name: 'Ask' }))
  expect((await screen.findByRole('alert')).textContent).toBe('Could not get an answer.')
})

test('parseEvents keeps an unfinished event for the next read', () => {
  const r = parseEvents('event: token\ndata: {"text":"a"}\n\nevent: tok')
  expect(r.events).toEqual([{ name: 'token', data: { text: 'a' } }])
  expect(r.rest).toBe('event: tok')
})
