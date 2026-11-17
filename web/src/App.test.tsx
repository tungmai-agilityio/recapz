import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import App from './App'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

function mount(path: string, me: Response, calls: string[] = [], logoutStatus = 200) {
  let loggedOut = false
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    calls.push(`${init?.method ?? 'GET'} ${url}`)
    if (url === '/auth/logout') { loggedOut = logoutStatus === 200; return new Response('', { status: logoutStatus }) }
    if (loggedOut) return new Response('', { status: 401 })
    if (url === '/api/me') return me.clone()
    return me.status === 401 ? new Response('', { status: 401 }) : new Response('[]')
  }))
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter initialEntries={[path]}><App /></MemoryRouter>
    </QueryClientProvider>,
  )
}

const signedIn = () => new Response(JSON.stringify({ email: 'ana@example.com', name: 'Ana', role: 'admin' }))

test('signed-in shell shows the rail, the account and the Meetings page', async () => {
  mount('/', signedIn())
  expect(await screen.findByText('No meetings yet. Upload a recording to get started.')).toBeTruthy()
  const rail = screen.getByRole('navigation', { name: 'Screens' })
  expect(rail.textContent).toContain('Work')
  expect(screen.getByRole('link', { name: 'Meetings' }).getAttribute('aria-current')).toBe('page')
  expect(screen.getByRole('link', { name: 'Ask' }).getAttribute('aria-current')).toBeNull()
  expect(screen.getByRole('link', { name: 'Recapz home' }).getAttribute('aria-current')).toBe('page')
  expect(await screen.findByText('ana@example.com')).toBeTruthy()
  expect(screen.queryByRole('menu')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: 'Account' }))
  expect(screen.getByRole('menu').textContent).toContain('Admin')
  expect(screen.getByRole('menuitem', { name: 'Sign out' })).toBeTruthy()
  expect(screen.getAllByTestId('v1-dot').length).toBe(3)
  fireEvent.keyDown(document, { key: 'Escape' })
  expect(screen.queryByRole('menu')).toBeNull()
})

test('the account menu links to Recording & privacy', async () => {
  mount('/', signedIn())
  fireEvent.click(await screen.findByRole('button', { name: 'Account' }))
  fireEvent.click(screen.getByRole('menuitem', { name: 'Recording & privacy' }))
  expect(await screen.findByRole('heading', { name: 'Recording & privacy' })).toBeTruthy()
})

test('the top bar has the Ask box, and Cmd+K or Ctrl+K opens Ask with the question focused', async () => {
  mount('/', signedIn())
  expect((await screen.findByRole('link', { name: /Ask about any meeting/ })).getAttribute('href')).toBe('/ask')
  fireEvent.keyDown(document, { key: 'k', metaKey: true })
  await waitFor(() => expect(document.activeElement).toBe(screen.getByLabelText('Question')))
  ;(document.activeElement as HTMLElement).blur()
  fireEvent.keyDown(document, { key: 'k', ctrlKey: true })
  await waitFor(() => expect(document.activeElement).toBe(screen.getByLabelText('Question')))
})

test('plain K does nothing', async () => {
  mount('/', signedIn())
  await screen.findByRole('navigation', { name: 'Screens' })
  fireEvent.keyDown(document, { key: 'k' })
  expect(screen.queryByLabelText('Question')).toBeNull()
})

test('the active rail item and breadcrumb follow the route', async () => {
  mount('/ask', signedIn())
  expect((await screen.findByRole('link', { name: 'Ask' })).getAttribute('aria-current')).toBe('page')
  expect(screen.getByRole('link', { name: 'Meetings' }).getAttribute('aria-current')).toBeNull()
  expect(screen.getByRole('link', { name: 'Recapz home' }).getAttribute('aria-current')).toBeNull()
  expect(document.querySelector('header')?.textContent).toContain('Ask')
  expect(document.querySelector('header')?.textContent).not.toContain('Meetings')
})

test('Recording & privacy is in the rail under Workspace, active on its route, with its breadcrumb', async () => {
  mount('/recording', signedIn())
  expect((await screen.findByRole('link', { name: 'Recording & privacy' })).getAttribute('aria-current')).toBe('page')
  expect(screen.getByRole('heading', { name: 'Recording & privacy' })).toBeTruthy()
  expect(document.querySelector('header')?.textContent).toContain('Workspace / Recording & privacy')
  expect(screen.getByRole('link', { name: 'Meetings' }).getAttribute('aria-current')).toBeNull()
})

test('Sign out posts to /auth/logout and returns to the sign-in card', async () => {
  const calls: string[] = []
  mount('/', signedIn(), calls)
  fireEvent.click(await screen.findByRole('button', { name: 'Account' }))
  fireEvent.click(screen.getByRole('menuitem', { name: 'Sign out' }))
  await waitFor(() => expect(calls).toContain('POST /auth/logout'))
  expect(await screen.findByText('Sign in to Recapz')).toBeTruthy()
  expect(screen.queryByRole('navigation', { name: 'Screens' })).toBeNull()
})

test('a failed sign-out keeps the shell and says so', async () => {
  mount('/', signedIn(), [], 403)
  fireEvent.click(await screen.findByRole('button', { name: 'Account' }))
  fireEvent.click(screen.getByRole('menuitem', { name: 'Sign out' }))
  expect((await screen.findByRole('alert')).textContent).toBe('Could not sign out.')
  expect(screen.getByRole('navigation', { name: 'Screens' })).toBeTruthy()
})

test('the report route shows its breadcrumb', async () => {
  mount('/meetings/m1', signedIn())
  await screen.findByRole('navigation', { name: 'Screens' })
  expect(document.querySelector('header')?.textContent).toContain('Meetings / Meeting report')
})

test('a 401 shows only the sign-in card, with no rail', async () => {
  mount('/', new Response('', { status: 401 }))
  expect(await screen.findByText('Sign in to Recapz')).toBeTruthy()
  expect(screen.queryByRole('navigation', { name: 'Screens' })).toBeNull()
})
