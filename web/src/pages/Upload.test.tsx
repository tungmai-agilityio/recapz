import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { Upload } from './Upload'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

function run(statuses: string[], calls: string[] = []) {
  let n = 0
  vi.stubGlobal('fetch', vi.fn(async (u: string, init?: RequestInit) => {
    calls.push(`${init?.method ?? 'GET'} ${u}`)
    if (init?.method === 'POST') {
      calls.push(`title=${(init.body as FormData).get('title')}`)
      return new Response(JSON.stringify({ id: 'm9' }), { status: 201 })
    }
    const status = statuses[Math.min(n++, statuses.length - 1)]
    return new Response(JSON.stringify({ id: 'm9', status, error: status === 'failed' ? 'bad audio' : null }))
  }))
  render(
    <QueryClientProvider client={new QueryClient()}><MemoryRouter><Upload /></MemoryRouter></QueryClientProvider>,
  )
}
const pick = (name: string) =>
  fireEvent.change(document.getElementById('file')!, { target: { files: [new File(['x'], name)] } })

test('a bad file disables Process recording', () => {
  run(['ready'])
  pick('notes.pdf')
  expect(screen.getByRole('alert').textContent).toMatch(/Unsupported/)
  expect((screen.getByRole('button', { name: 'Process recording' }) as HTMLButtonElement).disabled).toBe(true)
  pick('call.m4a')
  expect(screen.queryByRole('alert')).toBeNull()
  expect((screen.getByRole('button', { name: 'Process recording' }) as HTMLButtonElement).disabled).toBe(false)
  fireEvent.change(document.getElementById('file')!, { target: { files: [Object.defineProperty(new File(['x'], 'big.wav'), 'size', { value: 2 * 1024 ** 3 + 1 })] } })
  expect(screen.getByRole('alert').textContent).toMatch(/2 GB/)
})

test('progress follows the polled status and links to the meeting when ready', async () => {
  const calls: string[] = []
  run(['transcribing', 'ready'], calls)
  pick('call.mp3')
  fireEvent.change(screen.getByLabelText('Title'), { target: { value: ' Standup ' } })
  fireEvent.click(screen.getByRole('button', { name: 'Process recording' }))
  await waitFor(() => expect(document.querySelector('[data-state="run"]')?.textContent).toBe('Transcribing'))
  expect(calls).toContain('title=Standup')
  const link = await screen.findByRole('link', { name: 'Open the meeting' }, { timeout: 4000 })
  expect(link.getAttribute('href')).toBe('/meetings/m9')
  expect(document.querySelectorAll('[data-state="done"]').length).toBe(4)
})

test('failed shows the error; pressing the button again re-uploads', async () => {
  const calls: string[] = []
  run(['failed'], calls)
  pick('call.mp3')
  fireEvent.click(screen.getByRole('button', { name: 'Process recording' }))
  expect((await screen.findByText(/Processing failed: bad audio/)).textContent).toMatch(/try again/)
  fireEvent.click(screen.getByRole('button', { name: 'Process recording' }))
  await waitFor(() => expect(calls.filter((c) => c === 'POST /api/uploads').length).toBe(2))
})
