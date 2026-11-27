import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { Documents } from './Documents'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const usage = { used_bytes: 3 * 1024 * 1024 + 512 * 1024, cap_bytes: 500 * 1024 * 1024 }

function run(rows: unknown[], calls: string[] = [], used: unknown = usage) {
  vi.stubGlobal('fetch', vi.fn(async (u: string, init?: RequestInit) => {
    calls.push(`${init?.method ?? 'GET'} ${u}${typeof init?.body === 'string' ? ' ' + init.body : ''}`)
    if (init?.method === 'POST') return new Response(JSON.stringify({ id: 'd9' }), { status: 201 })
    if (init?.method) return new Response(null, { status: 204 })
    if (u === '/api/documents/usage') return used === null ? new Response('', { status: 500 }) : new Response(JSON.stringify(used))
    return new Response(JSON.stringify(rows))
  }))
  render(<MemoryRouter><QueryClientProvider client={new QueryClient()}><Documents /></QueryClientProvider></MemoryRouter>)
}
const added = new Date(2026, 10, 12, 9).getTime()
const rows = [
  { id: 'a1', name: 'plan.md', llm_ok: false, status: 'indexed', size_bytes: 1536, created_at: added },
  { id: 'b2', name: 'empty.txt', llm_ok: false, status: 'no text found', size_bytes: 12, created_at: added },
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

test('the explainer names no provider and no longer says audio never leaves the server', async () => {
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u === '/api/me' ? { email: 'a@example.com', name: null, role: 'member' } : []))))
  render(<QueryClientProvider client={new QueryClient()}><Documents /></QueryClientProvider>)
  expect(await screen.findByText('On: the passages that match a question you ask can be sent to the language model to write the answer.')).toBeTruthy()
  expect(document.body.textContent).not.toMatch(/provider|Deepgram/)
  expect(document.body.textContent).not.toMatch(/never leaves the server/)
})

test('each row shows its size and the day it was added', async () => {
  run([...rows, { id: 'c3', name: 'big.md', llm_ok: false, status: 'indexed', size_bytes: 2 * 1024 * 1024 + 300 * 1024, created_at: added }])
  const table = await screen.findByRole('table', { name: 'Documents' })
  const day = new Date(added).toLocaleDateString([], { year: 'numeric', month: 'short', day: 'numeric' })
  const row = (name: string) => within(table).getByText(name).closest('tr')!
  expect(within(row('plan.md')).getByText('2 KB')).toBeTruthy()
  expect(within(row('empty.txt')).getByText('12 B')).toBeTruthy()
  expect(within(row('big.md')).getByText('2.3 MB')).toBeTruthy()
  expect(within(row('plan.md')).getByText(day)).toBeTruthy()
  expect(within(table).getAllByRole('columnheader').map((h) => h.textContent)).toEqual(
    ['Name', 'Type', 'Size', 'Added', 'Status', 'Send to language model', ''])
})

test('the storage meter shows the bytes used against the cap, and is absent when the usage cannot be loaded', async () => {
  run(rows)
  const meter = await screen.findByRole('meter', { name: 'Storage used' })
  expect(meter.getAttribute('aria-valuenow')).toBe(String(usage.used_bytes))
  expect(meter.getAttribute('aria-valuemax')).toBe(String(usage.cap_bytes))
  expect(meter.getAttribute('aria-valuetext')).toBe('3.5 MB of 500 MB used')
  expect((meter.firstElementChild as HTMLElement).className).not.toContain('signal') // amber is for recording state and focus
  expect((meter.firstElementChild as HTMLElement).style.width).toBe('0.7%')
  expect(screen.getByText('3.5 MB of 500 MB used')).toBeTruthy()
  cleanup()
  run(rows, [], null)
  await screen.findByRole('table', { name: 'Documents' })
  expect(screen.queryByRole('meter')).toBeNull()
})

test('the meter is refreshed after a delete', async () => {
  const calls: string[] = []
  run(rows, calls)
  fireEvent.click((await screen.findAllByRole('button', { name: 'Delete' }))[0])
  fireEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Delete' }))
  await waitFor(() => expect(calls.filter((c) => c === 'GET /api/documents/usage').length).toBe(2))
})

test('each cell carries its column label so narrow screens can stack the row', async () => {
  run(rows)
  const table = await screen.findByRole('table', { name: 'Documents' })
  const cells = [...within(table).getAllByRole('row')[1].querySelectorAll('td')]
  expect(cells.map((c) => c.dataset.label)).toEqual(['Name', 'Type', 'Size', 'Added', 'Status', 'Send to language model', undefined])
})

test('each name links to its document page', async () => {
  run(rows)
  const table = await screen.findByRole('table', { name: 'Documents' })
  expect(within(table).getByRole('link', { name: 'plan.md' }).getAttribute('href')).toBe('/documents/a1')
  expect(within(table).getByRole('link', { name: 'empty.txt' }).getAttribute('href')).toBe('/documents/b2')
})
