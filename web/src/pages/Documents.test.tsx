import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import { Documents } from './Documents'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

function run(rows: unknown[], calls: string[] = []) {
  vi.stubGlobal('fetch', vi.fn(async (u: string, init?: RequestInit) => {
    calls.push(`${init?.method ?? 'GET'} ${u}${typeof init?.body === 'string' ? ' ' + init.body : ''}`)
    if (init?.method === 'POST') return new Response(JSON.stringify({ id: 'd9' }), { status: 201 })
    if (init?.method) return new Response(null, { status: 204 })
    return new Response(JSON.stringify(rows))
  }))
  render(<QueryClientProvider client={new QueryClient()}><Documents /></QueryClientProvider>)
}
const rows = [
  { id: 'a1', name: 'plan.md', llm_ok: false, status: 'indexed' },
  { id: 'b2', name: 'empty.txt', llm_ok: false, status: 'no text found' },
]

test('lists status; the switch is off by default and absent when there is no text', async () => {
  run(rows)
  const table = await screen.findByRole('table', { name: 'Documents' })
  expect(within(table).getAllByRole('row').length).toBe(3)
  expect(within(table).getByText('Indexed')).toBeTruthy()
  expect(within(table).getByText('No text found')).toBeTruthy()
  const sw = within(table).getAllByRole('switch')
  expect(sw.length).toBe(1)
  expect(sw[0].getAttribute('aria-checked')).toBe('false')
})

test('toggling sends PATCH llm_ok', async () => {
  const calls: string[] = []
  run(rows, calls)
  fireEvent.click(await screen.findByRole('switch'))
  await waitFor(() => expect(calls).toContain('PATCH /api/documents/a1 {"llm_ok":true}'))
})

test('delete asks first, then sends DELETE', async () => {
  const calls: string[] = []
  run(rows, calls)
  fireEvent.click((await screen.findAllByRole('button', { name: 'Delete' }))[0])
  expect(calls.filter((c) => c.startsWith('DELETE')).length).toBe(0)
  fireEvent.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Delete' }))
  await waitFor(() => expect(calls).toContain('DELETE /api/documents/a1'))
  await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull())
})

test('upload rejects a PDF locally and posts a Markdown file', async () => {
  const calls: string[] = []
  run([], calls)
  await screen.findByText('No documents yet.')
  fireEvent.change(document.getElementById('docs')!, { target: { files: [new File(['x'], 'a.pdf'), new File(['# hi'], 'n.md')] } })
  expect((await screen.findByRole('alert')).textContent).toBe('a.pdf: unsupported format. Use Markdown or TXT.')
  await waitFor(() => expect(calls.filter((c) => c === 'POST /api/documents').length).toBe(1))
})
