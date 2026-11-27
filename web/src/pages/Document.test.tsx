import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { Document } from './Document'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const added = new Date(2026, 10, 12, 9).getTime()
const detail = { id: 'a1', name: 'plan.md', kind: 'md', llm_ok: false, status: 'indexed', size_bytes: 1536, created_at: added, text: '# Plan\nline two', truncated: false }

function run(res: (u: string, init?: RequestInit) => Response, calls: string[] = []) {
  vi.stubGlobal('fetch', vi.fn(async (u: string, init?: RequestInit) => {
    calls.push(`${init?.method ?? 'GET'} ${u}${typeof init?.body === 'string' ? ' ' + init.body : ''}`)
    return res(u, init)
  }))
  render(
    <MemoryRouter initialEntries={['/documents/a1']}>
      <QueryClientProvider client={new QueryClient()}>
        <Routes>
          <Route path="/documents/:id" element={<Document />} />
          <Route path="/documents" element={<p>the list page</p>} />
        </Routes>
      </QueryClientProvider>
    </MemoryRouter>,
  )
}
const ok = (d: object = detail) => (_u: string, init?: RequestInit) => (init?.method ? new Response(null, { status: 204 }) : new Response(JSON.stringify(d)))

test('shows the details, the plain text and a download link to the original', async () => {
  run(ok())
  expect(await screen.findByRole('heading', { name: 'plan.md' })).toBeTruthy()
  expect(screen.getByText('Markdown')).toBeTruthy()
  expect(screen.getByText('2 KB')).toBeTruthy() // 1536 B, rounded like the list
  expect(screen.getByText('Indexed')).toBeTruthy()
  const pre = document.querySelector('pre')!
  expect(pre.textContent).toBe('# Plan\nline two') // plain text: the # is not rendered as a heading
  expect(screen.queryByText(/Only the first 1 MB/)).toBeNull()
  const dl = screen.getAllByRole('link', { name: 'Download' })
  expect(dl).toHaveLength(1)
  expect(dl[0].getAttribute('href')).toBe('/api/documents/a1/file')
  expect(dl[0].hasAttribute('download')).toBe(true)
  expect(screen.getByRole('link', { name: '← My documents' }).getAttribute('href')).toBe('/documents')
})

test('a cut text says only the first 1 MB is shown and offers the download', async () => {
  run(ok({ ...detail, truncated: true }))
  expect(await screen.findByText(/Only the first 1 MB is shown/)).toBeTruthy()
  expect(screen.getAllByRole('link', { name: 'Download' })).toHaveLength(2)
})

test('a missing or foreign document says not found, not an error', async () => {
  run(() => new Response('', { status: 404 }))
  expect(await screen.findByText('Document not found.')).toBeTruthy()
  expect(screen.queryByRole('alert')).toBeNull()
  expect(screen.getByRole('link', { name: '← My documents' })).toBeTruthy()
})

test('a server error shows an alert', async () => {
  run(() => new Response('', { status: 500 }))
  expect((await screen.findByRole('alert', undefined, { timeout: 4000 })).textContent).toBe('Could not load this document.')
})

test('the switch sends PATCH llm_ok and is absent when there is no text', async () => {
  const calls: string[] = []
  run(ok(), calls)
  fireEvent.click(await screen.findByRole('switch', { name: 'May be sent to the language model' }))
  await waitFor(() => expect(calls).toContain('PATCH /api/documents/a1 {"llm_ok":true}'))
  cleanup()
  run(ok({ ...detail, status: 'no text found', text: '' }))
  expect(await screen.findByText('This document has no text.')).toBeTruthy()
  expect(screen.queryByRole('switch')).toBeNull()
})

test('Delete asks first, then deletes and returns to the list', async () => {
  const calls: string[] = []
  run(ok(), calls)
  fireEvent.click(await screen.findByRole('button', { name: 'Delete' }))
  expect(calls).toEqual(['GET /api/documents/a1'])
  fireEvent.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Delete' }))
  expect(await screen.findByText('the list page')).toBeTruthy()
  expect(calls).toContain('DELETE /api/documents/a1')
})
