import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, screen } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { clock } from '../clock'
import { Report } from './Report'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const show = () =>
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter initialEntries={['/meetings/m1']}>
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
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? segs : meeting))))
  show()
  expect(await screen.findByText('Weekly sync')).toBeTruthy()
  expect(await screen.findByText('Xin chào')).toBeTruthy()
  expect(screen.getByText('Next topic')).toBeTruthy()
  expect(screen.getAllByRole('time').map((t) => t.textContent)).toEqual(['0:00', '1:05'])
})

test('says so when the meeting is not found', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 404 })))
  show()
  expect(await screen.findByText('Meeting not found.')).toBeTruthy()
})

test('formats clock times', () => {
  expect([clock(0), clock(59_999), clock(3_725_000)]).toEqual(['0:00', '0:59', '1:02:05'])
})
