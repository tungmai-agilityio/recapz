import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import App from './App'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

test('renders the app shell with the Meetings page', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('[]')))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter><App /></MemoryRouter>
    </QueryClientProvider>,
  )
  expect(screen.getByText('Recapz')).toBeTruthy()
  expect(await screen.findByText(/No meetings yet/)).toBeTruthy()
})
