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
const NOTICE = 'This meeting is recorded with Recapz. The audio is transcribed to write the meeting report.'

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
    if (url === '/api/me') return new Response(JSON.stringify({ email: 'a@b.c', name: 'Ana', role: 'member' }))
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

test('start waits for the consent box, and the copied notice names no provider', async () => {
  const d = await openDialog()
  fireEvent.change(within(d).getByLabelText('Title'), { target: { value: 'Client call' } })
  const start = within(d).getByRole('button', { name: 'Start recording' }) as HTMLButtonElement
  expect(start.disabled).toBe(true)
  expect(await within(d).findByText('I have told everyone in this meeting that it is recorded.')).toBeTruthy()
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

test('in the desktop app the dialog offers Meeting app and In-room, with the music hint, and no tab mode', async () => {
  vi.stubGlobal('__TAURI__', {})
  try {
    const d = await openDialog()
    expect(within(d).getAllByRole('radio').map((r) => r.closest('label')?.textContent)).toEqual([
      expect.stringContaining('Meeting app'),
      expect.stringContaining('In-room'),
    ])
    expect(within(d).getByText(/turn off other music and videos/)).toBeTruthy()
    expect(within(d).queryByText(/Meeting tab/)).toBeNull()
  } finally {
    vi.unstubAllGlobals()
  }
})

test('a missing permission in the desktop app offers a button that opens its System Settings pane', async () => {
  const invoke = vi.fn(async () => undefined)
  vi.stubGlobal('__TAURI__', { core: { invoke } })
  try {
    vi.mocked(startCapture).mockRejectedValue(new CaptureError('Recapz needs permission for Microphone.', 'microphone'))
    const d = await openDialog()
    fireEvent.change(within(d).getByLabelText('Title'), { target: { value: 'Client call' } })
    fireEvent.click(within(d).getByRole('checkbox'))
    fireEvent.click(within(d).getByRole('button', { name: 'Start recording' }))
    expect((await within(d).findByRole('alert')).textContent).toBe('Recapz needs permission for Microphone.')
    fireEvent.click(within(d).getByRole('button', { name: 'Open System Settings' }))
    expect(invoke.mock.calls).toEqual([['open_privacy_settings', { pane: 'microphone' }]])
  } finally {
    vi.unstubAllGlobals()
  }
})

const detected = (app: string) => act(() => { window.dispatchEvent(new CustomEvent('recapz-meeting-detected', { detail: { app } })) })
const ended = (app: string) => act(() => { window.dispatchEvent(new CustomEvent('recapz-meeting-ended', { detail: { app } })) })

/** The desktop app's prompt opens the dialog; `upcoming` and `meetings` are what the server returns meanwhile. */
async function fromPrompt(app: string, upcoming: unknown[] = [], meetings: unknown[] = []) {
  const real = vi.mocked(fetch).getMockImplementation()!
  vi.mocked(fetch).mockImplementation(async (url, init) => {
    if (url === '/api/meetings/upcoming') return new Response(JSON.stringify(upcoming))
    if (url === '/api/meetings') return new Response(JSON.stringify(meetings))
    return real(url, init)
  })
  show()
  await screen.findByRole('button', { name: 'Record from this browser' })
  detected(app)
  return screen.getByRole('dialog', { name: 'Record from this browser' })
}

test('the meeting prompt opens the dialog titled after the calendar event on now, with consent still unticked', async () => {
  vi.stubGlobal('__TAURI__', {})
  const now = Date.now()
  const event = (title: string, starts_at: number, ends_at: number) => ({ id: title, title, starts_at, ends_at, platform: null, join_url: null, auto_join: false, attendees: [] })
  const d = await fromPrompt('Zoom', [event('Weekly sync', now - 60_000, now + 60_000), event('Later', now + 3_600_000, now + 7_200_000)])
  await waitFor(() => expect((within(d).getByLabelText('Title') as HTMLInputElement).value).toBe('Weekly sync'))
  expect((within(d).getByRole('checkbox') as HTMLInputElement).checked).toBe(false)
  expect((within(d).getByRole('radio', { name: /Meeting app/ }) as HTMLInputElement).checked).toBe(true)
  expect(within(d).queryByRole('alert')).toBeNull()
  vi.unstubAllGlobals()
})

test('without a calendar event the title names the app and the time, and a busy notetaker is warned about', async () => {
  const d = await fromPrompt('Slack', [], [
    { id: 'b1', title: 'Board review', platform: 'meet', status: 'recording', started_at: 1, created_at: 1 },
    { id: 'b2', title: 'Old', platform: 'meet', status: 'done', started_at: 1, created_at: 1 },
  ])
  expect((within(d).getByLabelText('Title') as HTMLInputElement).value).toMatch(/^Slack call \d\d:\d\d$/)
  expect((await within(d).findByRole('alert')).textContent).toBe('The notetaker is already recording “Board review”. Record only if this is a different call.')
})

async function startFromPrompt(app: string, capture = fakeCapture()) {
  vi.mocked(startCapture).mockResolvedValue(capture)
  const d = await fromPrompt(app)
  fireEvent.click(within(d).getByRole('checkbox'))
  fireEvent.click(within(d).getByRole('button', { name: 'Start recording' }))
  await screen.findByRole('region', { name: 'Recorder' })
  return capture
}

test('a recording started from the prompt stops 10 s after its call ends', async () => {
  const capture = await startFromPrompt('Zoom')
  vi.useFakeTimers()
  try {
    ended('Teams')
    expect(screen.queryByRole('button', { name: 'Keep recording' })).toBeNull()
    ended('Zoom')
    expect(screen.getByText('The call in Zoom ended. Recording stops in 10 s.')).toBeTruthy()
    act(() => { vi.advanceTimersByTime(9_999) })
    expect(capture.stop).not.toHaveBeenCalled()
    act(() => { vi.advanceTimersByTime(1) })
    expect(vi.mocked(capture.stop).mock.calls.length).toBe(1)
    act(() => { vi.advanceTimersByTime(1_000) })
    expect(screen.queryByRole('region', { name: 'Recorder' })).toBeNull()
  } finally {
    vi.useRealTimers()
  }
})

test('Keep recording cancels the stop, and a later end does nothing', async () => {
  const capture = await startFromPrompt('Zoom')
  vi.useFakeTimers()
  try {
    ended('Zoom')
    fireEvent.click(screen.getByRole('button', { name: 'Keep recording' }))
    ended('Zoom')
    act(() => { vi.advanceTimersByTime(20_000) })
    expect(capture.stop).not.toHaveBeenCalled()
    expect(screen.queryByText(/Recording stops in/)).toBeNull()
  } finally {
    vi.useRealTimers()
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
  }
})

test('a recording started by hand never stops when a call ends, and no prompt dialog opens over it', async () => {
  const capture = await startTab()
  vi.useFakeTimers()
  try {
    ended('Zoom')
    act(() => { vi.advanceTimersByTime(20_000) })
    expect(capture.stop).not.toHaveBeenCalled()
    detected('Zoom')
    expect(screen.queryByRole('dialog')).toBeNull()
  } finally {
    vi.useRealTimers()
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
  }
})
