import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import { Meetings } from './Meetings'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const show = () =>
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <Meetings />
    </QueryClientProvider>,
  )

test('lists meetings from the API', async () => {
  const rows = [{ id: '1', title: 'Weekly sync', platform: 'upload', status: 'uploaded', started_at: null, created_at: 1 }]
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(rows))))
  show()
  expect(await screen.findByText('Weekly sync')).toBeTruthy()
})

test('shows sign-in when signed out', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 401 })))
  show()
  expect(await screen.findByText('Sign in with Google')).toBeTruthy()
})
