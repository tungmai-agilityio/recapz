import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { clock } from '../clock'
import { Report } from './Report'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const show = (path = '/meetings/m1') =>
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter initialEntries={[path]}>
        <Routes><Route path="/meetings/:id" element={<Report />} /></Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )

const meeting = { id: 'm1', title: 'Weekly sync', platform: 'upload', status: 'ready', language: 'vi+en', started_at: null, ended_at: null, created_at: 1, error: null }

test('shows the meeting and its transcript lines with times', async () => {
  const segs = [
    { id: 1, speaker_id: null, start_ms: 0, end_ms: 1000, text: 'Xin chào' },
    { id: 2, speaker_id: null, start_ms: 65000, end_ms: 70000, text: 'Next topic' },
  ]
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? segs : u.endsWith('/report') ? null : meeting))))
  show()
  expect(await screen.findByText('Weekly sync')).toBeTruthy()
  expect(await screen.findByText('Xin chào')).toBeTruthy()
  expect(screen.getByText('Next topic')).toBeTruthy()
  expect(screen.getAllByRole('time').map((t) => t.textContent)).toEqual(['0:00', '1:05'])
})

test('opens at the moment given in ?t= and highlights that line', async () => {
  const segs = [
    { id: 1, speaker_id: null, start_ms: 0, end_ms: 1000, text: 'Xin chào' },
    { id: 2, speaker_id: null, start_ms: 65000, end_ms: 70000, text: 'Next topic' },
  ]
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? segs : u.endsWith('/report') ? null : meeting))))
  const { container } = show('/meetings/m1?t=66000')
  const line = (await screen.findByText('Next topic')).closest('button')!
  await vi.waitFor(() => expect(line.getAttribute('aria-current')).toBe('true'))
  expect(screen.getByText('Xin chào').closest('button')!.getAttribute('aria-current')).toBeNull()
  expect(container.querySelector('audio')!.currentTime).toBe(66)
})

test('says so when the meeting is not found', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 404 })))
  show()
  expect(await screen.findByText('Meeting not found.')).toBeTruthy()
})

test('formats clock times', () => {
  expect([clock(0), clock(59_999), clock(3_725_000)]).toEqual(['0:00', '0:59', '1:02:05'])
})

test('clicking a transcript line seeks the audio and the playhead marks the current line', async () => {
  const segs = [
    { id: 1, speaker_id: null, start_ms: 0, end_ms: 65000, text: 'Xin chào' },
    { id: 2, speaker_id: null, start_ms: 65000, end_ms: 70000, text: 'Next topic' },
  ]
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? segs : u.endsWith('/report') ? null : meeting))))
  const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
  const { container } = show()
  fireEvent.click(await screen.findByText('Next topic'))
  const audio = container.querySelector('audio')!
  expect(audio.getAttribute('src')).toBe('/api/meetings/m1/audio')
  expect(audio.currentTime).toBe(65)
  expect(play).toHaveBeenCalledTimes(1)
  expect(screen.getByText('Next topic').closest('button')?.getAttribute('aria-current')).toBe('true')
  expect(screen.getByText('Xin chào').closest('button')?.getAttribute('aria-current')).toBeNull()
  play.mockRestore()
})

test('shows the report sections and chapter and action-item times seek the audio', async () => {
  const report = {
    summary: 'We agreed the plan.',
    decisions: ['Ship on Friday'],
    open_questions: ['Who owns QA?'],
    chapters: [{ start_ms: 65000, title: 'Planning' }],
    action_items: [{ id: 'a1', text: 'Write the notes', due_date: '2026-12-01', done: false, source_ms: 125000 }],
  }
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? [] : u.endsWith('/report') ? report : meeting))))
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
  const { container } = show()
  expect(await screen.findByText('We agreed the plan.')).toBeTruthy()
  expect(screen.getByText('Ship on Friday')).toBeTruthy()
  expect(screen.getByText('Who owns QA?')).toBeTruthy()
  expect(screen.getByText('due 2026-12-01')).toBeTruthy()
  const audio = container.querySelector('audio')!
  fireEvent.click(screen.getByText('Planning'))
  expect(audio.currentTime).toBe(65)
  fireEvent.click(screen.getByText('2:05'))
  expect(audio.currentTime).toBe(125)
})

test('tells the member the report is on its way while the meeting is being reported', async () => {
  const m = { ...meeting, status: 'reporting' }
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? [] : u.endsWith('/report') ? null : m))))
  show()
  expect(await screen.findByText('The report will appear when processing finishes.')).toBeTruthy()
})
