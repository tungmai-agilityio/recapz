import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { NextUp } from './NextUp'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const show = () =>
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter><NextUp /></MemoryRouter>
    </QueryClientProvider>,
  )
const ev = (o = {}) => ({ id: 'e1', title: 'Design review', starts_at: Date.now() + 3_600_000, ends_at: Date.now() + 7_200_000, platform: 'meet', has_link: true, auto_join: false, ...o })
const api = (connected: boolean, events: unknown[], calls: [string, RequestInit | undefined][] = []) =>
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    calls.push([url, init])
    if (url === '/api/calendar') return new Response(JSON.stringify({ connected, needs_reconnect: false }))
    if (url === '/api/meetings/upcoming') return new Response(JSON.stringify(events))
    return new Response(null, { status: 204 })
  }))

test('lists upcoming meetings with title, platform and an auto-join switch', async () => {
  api(true, [ev(), ev({ id: 'e2', title: 'Hallway chat', platform: null, has_link: false })])
  show()
  expect(await screen.findByText('Design review')).toBeTruthy()
  expect(screen.getAllByText('meet')).toHaveLength(1)
  expect(screen.getByRole('switch', { name: 'Auto-join Design review' }).getAttribute('aria-checked')).toBe('false')
  expect(screen.getAllByRole('switch')).toHaveLength(1)
  expect(screen.getAllByText('Auto-join')).toHaveLength(1)
  expect(screen.getByText('No link')).toBeTruthy()
})

test('flipping the switch sends PATCH with auto_join', async () => {
  const calls: [string, RequestInit | undefined][] = []
  api(true, [ev()], calls)
  show()
  fireEvent.click(await screen.findByRole('switch', { name: 'Auto-join Design review' }))
  await waitFor(() => expect(calls.some(([u, i]) => u === '/api/calendar-events/e1' && i?.method === 'PATCH' && i.body === '{"auto_join":true}')).toBe(true))
})

test('empty state points to Recording & privacy', async () => {
  api(true, [])
  show()
  expect(await screen.findByText(/No upcoming meetings/)).toBeTruthy()
  expect(screen.getByRole('link', { name: 'Recording & privacy' }).getAttribute('href')).toBe('/recording')
})

test('not connected: says so and does not ask for events', async () => {
  const calls: [string, RequestInit | undefined][] = []
  api(false, [], calls)
  show()
  expect(await screen.findByText(/calendar is not connected/)).toBeTruthy()
  expect(calls.map(([u]) => u)).toEqual(['/api/calendar'])
})

test('flipping the switch refetches the list', async () => {
  const calls: [string, RequestInit | undefined][] = []
  api(true, [ev()], calls)
  show()
  fireEvent.click(await screen.findByRole('switch', { name: 'Auto-join Design review' }))
  await waitFor(() => expect(calls.filter(([u, i]) => u === '/api/meetings/upcoming' && !i?.method)).toHaveLength(2))
})

test('a failed events request shows an error', async () => {
  vi.stubGlobal('fetch', vi.fn(async (url: string) =>
    url === '/api/calendar' ? new Response(JSON.stringify({ connected: true, needs_reconnect: false })) : new Response('boom', { status: 500 })))
  show()
  expect((await screen.findByRole('alert')).textContent).toBe('Could not load upcoming meetings.')
})
