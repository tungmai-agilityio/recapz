import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import { Members } from './Members'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const data = {
  members: [
    { id: 'u1', email: 'ana@example.com', name: 'Ana', role: 'admin', meetings: 4 },
    { id: 'u2', email: 'bo@example.com', name: null, role: 'member', meetings: 0 },
  ],
  invites: [{ email: 'cy@example.com', role: 'member', created_at: 1 }],
}

function run(calls: string[] = [], fail?: Response, role = 'admin', email = 'off') {
  vi.stubGlobal('fetch', vi.fn(async (u: string, init?: RequestInit) => {
    calls.push(`${init?.method ?? 'GET'} ${u}${typeof init?.body === 'string' ? ' ' + init.body : ''}`)
    if (u === '/api/me') return new Response(JSON.stringify({ email: 'ana@example.com', name: 'Ana', role }))
    if (init?.method && fail) return fail
    if (init?.method === 'POST') return new Response(JSON.stringify({ link: 'https://x.test/', email }), { status: 201 })
    if (init?.method) return new Response(null, { status: 204 })
    return new Response(JSON.stringify(data))
  }))
  render(<QueryClientProvider client={new QueryClient()}><Members /></QueryClientProvider>)
}

test('lists members and the pending invite; you cannot remove yourself', async () => {
  run()
  const table = await screen.findByRole('table', { name: 'Members' })
  expect(within(table).getAllByRole('row').length).toBe(4)
  expect((within(table).getByLabelText('Role of Ana') as HTMLSelectElement).value).toBe('admin')
  expect(within(table).getByText('Pending')).toBeTruthy()
  expect(within(table).queryByRole('button', { name: 'Remove Ana' })).toBeNull()
  expect(within(table).getByRole('button', { name: 'Remove bo@example.com' })).toBeTruthy()
  expect(within(table).getByRole('button', { name: 'Cancel invite for cy@example.com' })).toBeTruthy()
})

test('a member gets a notice and the members API is never requested', async () => {
  const calls: string[] = []
  run(calls, undefined, 'member')
  expect(await screen.findByText('Only an admin can manage members.')).toBeTruthy()
  expect(calls).toEqual(['GET /api/me'])
})

test('changing a role sends PATCH', async () => {
  const calls: string[] = []
  run(calls)
  fireEvent.change(await screen.findByLabelText('Role of bo@example.com'), { target: { value: 'admin' } })
  await waitFor(() => expect(calls).toContain('PATCH /api/members/u2 {"role":"admin"}'))
})

test('remove confirms inline, then sends DELETE; Keep sends nothing', async () => {
  const calls: string[] = []
  run(calls)
  fireEvent.click(await screen.findByRole('button', { name: 'Remove bo@example.com' }))
  fireEvent.click(screen.getByRole('button', { name: 'Keep' }))
  expect(calls.filter((c) => c.startsWith('DELETE')).length).toBe(0)
  fireEvent.click(screen.getByRole('button', { name: 'Remove bo@example.com' }))
  fireEvent.click(screen.getByRole('button', { name: 'Confirm: Remove bo@example.com' }))
  await waitFor(() => expect(calls).toContain('DELETE /api/members/u2'))
})

test('cancel invite confirms, then deletes by email', async () => {
  const calls: string[] = []
  run(calls)
  fireEvent.click(await screen.findByRole('button', { name: 'Cancel invite for cy@example.com' }))
  fireEvent.click(screen.getByRole('button', { name: 'Confirm: Cancel invite for cy@example.com' }))
  await waitFor(() => expect(calls).toContain('DELETE /api/members/invites/cy%40example.com'))
})

test('invite posts email and role, shows the link and Copy link copies it', async () => {
  const calls: string[] = []
  const write = vi.fn(async () => {})
  vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText: write } })
  run(calls)
  fireEvent.change(await screen.findByLabelText('Email'), { target: { value: ' dee@example.com ' } })
  fireEvent.change(screen.getByLabelText('Role'), { target: { value: 'admin' } })
  fireEvent.click(screen.getByRole('button', { name: 'Invite' }))
  await waitFor(() => expect(calls).toContain('POST /api/members/invites {"email":"dee@example.com","role":"admin"}'))
  expect((await screen.findByRole('status')).textContent).toContain('Invite saved. Email is not set up, so share this link:')
  fireEvent.click(await screen.findByRole('button', { name: 'Copy link' }))
  await waitFor(() => expect(write).toHaveBeenCalledWith('https://x.test/'))
})

test.each([
  ['sent', 'Invite emailed to dee@example.com. You can also share this link: https://x.test/'],
  ['failed', 'Invite saved, but the email to dee@example.com could not be sent. Share this link: https://x.test/'],
])('an invite whose email was %s says so', async (email, text) => {
  run([], undefined, 'admin', email)
  fireEvent.change(await screen.findByLabelText('Email'), { target: { value: 'dee@example.com' } })
  fireEvent.click(screen.getByRole('button', { name: 'Invite' }))
  expect((await screen.findByRole('status')).textContent).toBe(`${text}Copy link`)
})

test('a bad email is refused locally; a server refusal shows next to the field', async () => {
  const calls: string[] = []
  run(calls, new Response('already a member of this workspace', { status: 409 }))
  fireEvent.change(await screen.findByLabelText('Email'), { target: { value: 'nope' } })
  fireEvent.click(screen.getByRole('button', { name: 'Invite' }))
  expect((await screen.findByRole('alert')).textContent).toBe('Enter a valid email.')
  expect(calls.filter((c) => c.startsWith('POST')).length).toBe(0)
  fireEvent.change(screen.getByLabelText('Email'), { target: { value: 'bo@example.com' } })
  fireEvent.click(screen.getByRole('button', { name: 'Invite' }))
  await waitFor(() => expect(screen.getByRole('alert').textContent).toBe('already a member of this workspace'))
})
