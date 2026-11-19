import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import App from '../App'
import { CaptureError, startCapture, type Capture } from '../capture/capture'
import { FrameSocket } from '../capture/socket'
import { ToastProvider } from '../ui/Toast'

vi.mock('../capture/capture', async (orig) => ({ ...(await orig<object>()), startCapture: vi.fn() }))
vi.mock('../capture/socket', () => ({ FrameSocket: vi.fn(function (this: { discard: () => void }) { this.discard = discard }) }))
const discard = vi.fn()

const meeting = { id: 'm1', title: 'Client call', platform: 'browser', status: 'recording', language: 'vi', started_at: 1, ended_at: null, created_at: 1, error: null, speakers: [] }
const NOTICE = 'This meeting is recorded with Recapz. The audio is sent to Deepgram to be transcribed.'

function fakeCapture(): Capture {
  return { mode: 'tab', state: 'recording', levels: { mic: 0.5, tab: 0.25 }, tabEnded: false, dropped: 0, stop: vi.fn(), discard: vi.fn() }
}

let posts: unknown[] = []
beforeEach(() => {
  posts = []
  Object.defineProperty(navigator, 'userAgentData', { value: { mobile: false }, configurable: true })
  Object.defineProperty(navigator, 'mediaDevices', { value: { getDisplayMedia: vi.fn() }, configurable: true })
  Object.defineProperty(navigator, 'clipboard', { value: { writeText: vi.fn(async () => {}) }, configurable: true })
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    if (url === '/api/me') return new Response(JSON.stringify({ email: 'a@b.c', name: 'Ana', role: 'member', speech_provider: 'Deepgram' }))
    if (url === '/api/meetings') return new Response(JSON.stringify([]))
    if (url === '/api/recordings') { posts.push(JSON.parse(init!.body as string)); return new Response(JSON.stringify({ id: 'm1' }), { status: 201 }) }
    if (url === '/api/meetings/m1') return new Response(JSON.stringify(meeting))
    if (url === '/api/meetings/m1/live') return new Response(new ReadableStream())
    return new Response('', { status: 404 })
  }))
})
afterEach(() => {
  cleanup(); vi.unstubAllGlobals(); vi.clearAllMocks()
  for (const k of ['userAgentData', 'mediaDevices', 'clipboard']) Reflect.deleteProperty(navigator, k)
})

const show = (at = '/') =>
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ToastProvider><MemoryRouter initialEntries={[at]}><App /></MemoryRouter></ToastProvider>
    </QueryClientProvider>,
  )

async function openDialog() {
  show()
  fireEvent.click(await screen.findByRole('button', { name: 'Record from this browser' }))
  return screen.getByRole('dialog', { name: 'Record from this browser' })
}

async function startTab(capture = fakeCapture()) {
  vi.mocked(startCapture).mockResolvedValue(capture)
  const d = await openDialog()
  fireEvent.change(within(d).getByLabelText('Title'), { target: { value: ' Client call ' } })
  fireEvent.click(within(d).getByRole('checkbox'))
  fireEvent.click(within(d).getByRole('button', { name: 'Start recording' }))
  await screen.findByRole('region', { name: 'Recorder' })
  return capture
}

test('start waits for the consent box, and the copied notice names the speech provider', async () => {
  const d = await openDialog()
  fireEvent.change(within(d).getByLabelText('Title'), { target: { value: 'Client call' } })
  const start = within(d).getByRole('button', { name: 'Start recording' }) as HTMLButtonElement
  expect(start.disabled).toBe(true)
  expect(await within(d).findByText('I have told everyone in this meeting that it is recorded and sent to Deepgram.')).toBeTruthy()
  fireEvent.click(within(d).getByRole('checkbox'))
  expect(start.disabled).toBe(false)
  fireEvent.click(within(d).getByRole('button', { name: 'Copy for the call chat' }))
  expect(vi.mocked(navigator.clipboard.writeText).mock.calls).toEqual([[NOTICE]])
})

test('a browser without tab audio is pointed to Upload and cannot start', async () => {
  Reflect.deleteProperty(navigator, 'userAgentData')
  const d = await openDialog()
  expect(within(d).getByRole('link', { name: 'upload the recording' }).getAttribute('href')).toBe('/upload')
  expect(within(d).queryByRole('button', { name: 'Start recording' })).toBeNull()
})

test('start creates the recording, streams to its socket and opens the live view with the recorder bar', async () => {
  const capture = await startTab()
  expect(posts).toEqual([{ title: 'Client call', language: 'vi', mode: 'tab' }])
  expect(vi.mocked(startCapture).mock.calls).toEqual([['tab', '/api/meetings/m1/record']])
  expect(await screen.findByRole('heading', { name: 'Client call' })).toBeTruthy()
  expect(screen.getByLabelText('Elapsed time').textContent).toBe('0:00')
  expect(screen.getAllByRole('meter').map((m) => (m as HTMLMeterElement).value)).toEqual([0.5, 0.25])
  fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
  expect(vi.mocked(capture.stop).mock.calls.length).toBe(1)
  expect(capture.discard).not.toHaveBeenCalled()
  expect(screen.queryByRole('region', { name: 'Recorder' })).toBeNull()
})

test('a refused share shows why and discards the meeting it created', async () => {
  vi.mocked(startCapture).mockRejectedValue(new CaptureError('Share a tab with audio.'))
  const d = await openDialog()
  fireEvent.change(within(d).getByLabelText('Title'), { target: { value: 'Client call' } })
  fireEvent.click(within(d).getByRole('checkbox'))
  fireEvent.click(within(d).getByRole('button', { name: 'Start recording' }))
  expect((await within(d).findByRole('alert')).textContent).toBe('Share a tab with audio.')
  expect(vi.mocked(FrameSocket).mock.calls).toEqual([['ws://localhost:3000/api/meetings/m1/record']])
  expect(discard.mock.calls.length).toBe(1)
})

test('stop and discard asks first, then discards and goes back to Meetings', async () => {
  const capture = await startTab()
  fireEvent.click(screen.getByRole('button', { name: 'Stop and discard' }))
  expect(capture.discard).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'Discard recording' }))
  expect(vi.mocked(capture.discard).mock.calls.length).toBe(1)
  expect(capture.stop).not.toHaveBeenCalled()
  expect(await screen.findByRole('heading', { name: 'Meetings' })).toBeTruthy()
})

test('warns when the shared tab stops sending audio, and before leaving the page', async () => {
  const capture = await startTab()
  expect(screen.queryByText(/stopped sharing its audio/)).toBeNull()
  act(() => { capture.tabEnded = true; capture.onChange?.() })
  expect(screen.getByText('The meeting tab stopped sharing its audio. Only your microphone is recorded now.')).toBeTruthy()
  const leave = new Event('beforeunload', { cancelable: true })
  window.dispatchEvent(leave)
  expect(leave.defaultPrevented).toBe(true)
  fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
})

test('another tab watching the same live view has no recorder bar', async () => {
  show('/meetings/m1/live')
  expect(await screen.findByRole('heading', { name: 'Client call' })).toBeTruthy()
  await waitFor(() => expect(screen.queryByRole('region', { name: 'Recorder' })).toBeNull())
})
