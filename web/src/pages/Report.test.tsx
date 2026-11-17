import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router'
import { afterEach, expect, test, vi } from 'vitest'
import { clock } from '../clock'
import { ToastProvider } from '../ui/Toast'
import { Report } from './Report'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const show = (path = '/meetings/m1') =>
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ToastProvider>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route path="/meetings/:id" element={<Report />} />
            <Route path="/" element={<p>Meetings home</p>} />
          </Routes>
        </MemoryRouter>
      </ToastProvider>
    </QueryClientProvider>,
  )

const meeting = { id: 'm1', title: 'Weekly sync', platform: 'upload', status: 'ready', language: 'vi+en', started_at: null, ended_at: null, created_at: 1, error: null }

test('shows the meeting and its transcript lines with times', async () => {
  const segs = [
    { id: 1, speaker_id: null, start_ms: 0, end_ms: 1000, text: 'Xin chào' },
    { id: 2, speaker_id: null, start_ms: 65000, end_ms: 70000, text: 'Next topic' },
  ]
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? segs : u.endsWith('/report') ? null : meeting))))
  show()
  expect(await screen.findByText('Weekly sync')).toBeTruthy()
  fireEvent.click(screen.getByRole('tab', { name: 'Transcript' }))
  expect(await screen.findByText('Xin chào')).toBeTruthy()
  expect(screen.getByText('Next topic')).toBeTruthy()
  expect(screen.getAllByRole('time').map((t) => t.textContent)).toEqual(['0:00', '1:05'])
})

test('opens at the moment given in ?t= and highlights that line', async () => {
  const segs = [
    { id: 1, speaker_id: null, start_ms: 0, end_ms: 1000, text: 'Xin chào' },
    { id: 2, speaker_id: null, start_ms: 65000, end_ms: 70000, text: 'Next topic' },
  ]
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? segs : u.endsWith('/report') ? null : meeting))))
  const { container } = show('/meetings/m1?t=66000')
  const line = (await screen.findByText('Next topic')).closest('button')!
  await vi.waitFor(() => expect(line.getAttribute('aria-current')).toBe('true'))
  expect(screen.getByText('Xin chào').closest('button')!.getAttribute('aria-current')).toBeNull()
  expect(container.querySelector('audio')!.currentTime).toBe(66)
})

test('says so when the meeting is not found', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 404 })))
  show()
  expect(await screen.findByText('Meeting not found.')).toBeTruthy()
})

test('formats clock times', () => {
  expect([clock(0), clock(59_999), clock(3_725_000)]).toEqual(['0:00', '0:59', '1:02:05'])
})

test('clicking a transcript line seeks the audio and the playhead marks the current line', async () => {
  const segs = [
    { id: 1, speaker_id: null, start_ms: 0, end_ms: 65000, text: 'Xin chào' },
    { id: 2, speaker_id: null, start_ms: 65000, end_ms: 70000, text: 'Next topic' },
  ]
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? segs : u.endsWith('/report') ? null : meeting))))
  const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
  const { container } = show()
  fireEvent.click(await screen.findByRole('tab', { name: 'Transcript' }))
  fireEvent.click(await screen.findByText('Next topic'))
  const audio = container.querySelector('audio')!
  expect(audio.getAttribute('src')).toBe('/api/meetings/m1/audio')
  expect(audio.currentTime).toBe(65)
  expect(play).toHaveBeenCalledTimes(1)
  expect(screen.getByText('Next topic').closest('button')?.getAttribute('aria-current')).toBe('true')
  expect(screen.getByText('Xin chào').closest('button')?.getAttribute('aria-current')).toBeNull()
  play.mockRestore()
})

test('shows the report sections and chapter and action-item times seek the audio', async () => {
  const report = {
    summary: 'We agreed the plan.',
    decisions: ['Ship on Friday'],
    open_questions: ['Who owns QA?'],
    chapters: [{ start_ms: 65000, title: 'Planning' }],
    action_items: [{ id: 'a1', text: 'Write the notes', due_date: '2026-12-01', done: false, source_ms: 125000 }],
  }
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? [] : u.endsWith('/report') ? report : meeting))))
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
  const { container } = show()
  expect(await screen.findByText('We agreed the plan.')).toBeTruthy()
  expect(screen.getByText('Ship on Friday')).toBeTruthy()
  expect(screen.getByText('Who owns QA?')).toBeTruthy()
  expect(screen.getByText('due 2026-12-01')).toBeTruthy()
  expect(screen.getByText('Action items · 0/1 done')).toBeTruthy()
  const audio = container.querySelector('audio')!
  fireEvent.click(screen.getByText('2:05'))
  expect(audio.currentTime).toBe(125)
  fireEvent.click(screen.getByText('Planning'))
  expect(audio.currentTime).toBe(65)
  expect(screen.getByRole('tab', { name: 'Transcript' }).getAttribute('aria-selected')).toBe('true')
})

test('transport: play button and Space toggle the audio, clicking the ribbon seeks', async () => {
  const segs = [
    { id: 1, speaker_id: 's1', start_ms: 0, end_ms: 60000, text: 'One' },
    { id: 2, speaker_id: 's2', start_ms: 60000, end_ms: 100000, text: 'Two' },
  ]
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? segs : u.endsWith('/report') ? null : meeting))))
  const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
  const { container } = show()
  const slider = await screen.findByRole('slider', { name: 'Speaker ribbon' })
  expect(screen.getByText('/ 1:40')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Play' }))
  fireEvent.keyDown(document.body, { key: ' ' })
  expect(play).toHaveBeenCalledTimes(2)
  slider.getBoundingClientRect = () => ({ left: 0, width: 200 }) as DOMRect
  fireEvent.click(slider, { clientX: 100 })
  expect(container.querySelector('audio')!.currentTime).toBe(50)
  expect(screen.getByTestId('playhead').style.left).toBe('50%')
  play.mockRestore()
})

test('says there is no audio when the file cannot be loaded', async () => {
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? [] : u.endsWith('/report') ? null : meeting))))
  const { container } = show()
  await screen.findByText('Weekly sync')
  fireEvent.error(container.querySelector('audio')!)
  expect(await screen.findByText('No audio is available for this meeting yet.')).toBeTruthy()
  expect(screen.queryByRole('button', { name: 'Play' })).toBeNull()
})

test('shows a failed meeting with its error and no report', async () => {
  const m = { ...meeting, status: 'failed', error: 'transcription failed' }
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? [] : u.endsWith('/report') ? null : m))))
  show()
  expect((await screen.findByRole('alert')).textContent).toBe('transcription failed')
  expect(screen.getByText('No report, because processing failed.')).toBeTruthy()
})

test('tells the member the report is on its way while the meeting is being reported', async () => {
  const m = { ...meeting, status: 'reporting' }
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? [] : u.endsWith('/report') ? null : m))))
  show()
  expect(await screen.findByText('The report will appear when processing finishes.')).toBeTruthy()
})

const deleteSetup = (del: () => Response) => {
  const f = vi.fn(async (u: string, init?: RequestInit) =>
    init?.method === 'DELETE' ? del() : new Response(JSON.stringify(u.endsWith('/transcript') ? [] : u.endsWith('/report') ? null : meeting)))
  vi.stubGlobal('fetch', f)
  const deletes = () => f.mock.calls.filter(([, i]) => i?.method === 'DELETE')
  show()
  return deletes
}
const openDialog = async () => {
  fireEvent.click(await screen.findByRole('button', { name: 'Meeting actions' }))
  fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }))
  return screen.getByRole('alertdialog', { name: 'Delete this meeting?' })
}

test('Cancel closes the delete dialog without calling the API', async () => {
  const deletes = deleteSetup(() => new Response(null, { status: 204 }))
  await openDialog()
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(screen.queryByRole('alertdialog')).toBeNull()
  expect(deletes()).toHaveLength(0)
})

test('confirming deletes that meeting, shows a toast and leaves the page', async () => {
  const deletes = deleteSetup(() => new Response(null, { status: 204 }))
  await openDialog()
  fireEvent.click(screen.getByRole('button', { name: 'Delete meeting' }))
  expect(await screen.findByText('Meetings home')).toBeTruthy()
  expect(screen.getByRole('status').textContent).toBe('Meeting deleted')
  expect(deletes()).toHaveLength(1)
  expect(deletes()[0][0]).toBe('/api/meetings/m1')
})

test('a failed delete keeps the dialog open and shows the server message in it', async () => {
  const deletes = deleteSetup(() => new Response('meeting is still processing', { status: 409 }))
  const d = await openDialog()
  fireEvent.click(screen.getByRole('button', { name: 'Delete meeting' }))
  expect((await screen.findByRole('alert')).textContent).toBe('meeting is still processing')
  expect(d.isConnected).toBe(true)
  expect(deletes()).toHaveLength(1)
  expect(screen.queryByText('Meetings home')).toBeNull()
})

const renameSetup = (patch: (body: string) => Response) => {
  const f = vi.fn(async (u: string, init?: RequestInit) =>
    init?.method === 'PATCH' ? patch(String(init.body)) : new Response(JSON.stringify(u.endsWith('/transcript') ? [] : u.endsWith('/report') ? null : meeting)))
  vi.stubGlobal('fetch', f)
  show()
  return () => f.mock.calls.filter(([, i]) => i?.method === 'PATCH')
}
const startRename = async () => {
  fireEvent.click(await screen.findByRole('button', { name: 'Meeting actions' }))
  fireEvent.click(screen.getByRole('menuitem', { name: 'Rename' }))
  return screen.getByRole('textbox', { name: 'Meeting title' }) as HTMLInputElement
}

test('Enter saves the trimmed title and the header shows it', async () => {
  const patches = renameSetup((b) => new Response(JSON.stringify({ title: JSON.parse(b).title })))
  const input = await startRename()
  expect(input.value).toBe('Weekly sync')
  fireEvent.change(input, { target: { value: '  Q3 planning ' } })
  fireEvent.submit(input.form!)
  expect((await screen.findByRole('heading', { level: 1 })).textContent).toBe('Q3 planning')
  expect(screen.queryByRole('textbox')).toBeNull()
  expect(patches()).toHaveLength(1)
  expect(patches()[0][0]).toBe('/api/meetings/m1')
  expect(patches()[0][1]?.body).toBe('{"title":"Q3 planning"}')
})

test('Escape and Cancel leave the title alone without calling the API', async () => {
  const patches = renameSetup(() => new Response('{}'))
  const input = await startRename()
  fireEvent.change(input, { target: { value: 'Other' } })
  fireEvent.keyDown(input, { key: 'Escape' })
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Weekly sync')
  await startRename()
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Weekly sync')
  expect(patches()).toHaveLength(0)
})

test('a blank title cannot be saved', async () => {
  const patches = renameSetup(() => new Response('{}'))
  const input = await startRename()
  fireEvent.change(input, { target: { value: '   ' } })
  expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(true)
  fireEvent.submit(input.form!)
  expect(patches()).toHaveLength(0)
})

test('a failed rename keeps the editor open with the server message', async () => {
  const patches = renameSetup(() => new Response('only the owner or an admin can rename', { status: 403 }))
  const input = await startRename()
  fireEvent.change(input, { target: { value: 'Nope' } })
  fireEvent.submit(input.form!)
  expect((await screen.findByRole('alert')).textContent).toBe('only the owner or an admin can rename')
  expect((screen.getByRole('textbox', { name: 'Meeting title' }) as HTMLInputElement).value).toBe('Nope')
  expect(patches()).toHaveLength(1)
})

const people = [
  { id: 1, speaker_id: 'a', speaker_name: 'Khoa', start_ms: 0, end_ms: 10000, text: 'Hello' },
  { id: 2, speaker_id: 'b', speaker_name: 'Sarah', start_ms: 10000, end_ms: 40000, text: 'Hi there' },
  { id: 3, speaker_id: 'a', speaker_name: 'Khoa', start_ms: 40000, end_ms: 50000, text: 'Bye' },
  { id: 4, speaker_id: null, speaker_name: null, start_ms: 50000, end_ms: 100000, text: 'Unlabelled' },
]
const withReport = { summary: 'Plan agreed', decisions: [], open_questions: [], chapters: [], action_items: [] }
const serve = (segs: unknown[]) =>
  vi.stubGlobal('fetch', vi.fn(async (u: string) => new Response(JSON.stringify(u.endsWith('/transcript') ? segs : u.endsWith('/report') ? withReport : meeting))))
const swatch = (e: Element) => e.className.match(/bg-speaker-\d/)![0]

test('shows speaker names on lines, faces in the header and talk time with one colour per person', async () => {
  serve(people)
  show()
  await screen.findByText('Plan agreed')
  const faces = screen.getAllByRole('img')
  expect(faces.map((f) => [f.getAttribute('aria-label'), swatch(f)])).toEqual([['Khoa', 'bg-speaker-1'], ['Sarah', 'bg-speaker-2']])
  const rows = screen.getByRole('heading', { name: 'Talk time' }).parentElement!.querySelectorAll('li')
  expect([...rows].map((r) => r.textContent)).toEqual(['Sarah30%', 'Khoa20%'])
  const bars = screen.getAllByTestId('talk-bar')
  expect(bars.map((b) => swatch(b))).toEqual(['bg-speaker-2', 'bg-speaker-1'])
  expect(bars[0].style.width).toBe('100%')
  expect(parseFloat(bars[1].style.width)).toBeCloseTo(66.667, 2)
  fireEvent.click(screen.getByRole('tab', { name: 'Transcript' }))
  expect(screen.getByText('Hi there').closest('button')!.textContent).toBe('0:10SarahHi there')
  expect(screen.getByText('Unlabelled').closest('button')!.textContent).toBe('0:50Unlabelled')
})

test('hides Talk time and faces when no segment has a speaker', async () => {
  serve([{ id: 1, speaker_id: null, speaker_name: null, start_ms: 0, end_ms: 1000, text: 'x' }])
  show()
  await screen.findByText('Plan agreed')
  expect(screen.queryByRole('heading', { name: 'Talk time' })).toBeNull()
  expect(screen.queryAllByRole('img')).toEqual([])
})

test('labels a speaker with no name as Unknown', async () => {
  serve([{ id: 1, speaker_id: 'z', speaker_name: null, start_ms: 0, end_ms: 1000, text: 'x' }])
  show()
  await screen.findByText('Plan agreed')
  expect(screen.getAllByText('Unknown')).toHaveLength(1)
  expect(screen.getByRole('img').getAttribute('aria-label')).toBe('Unknown')
})
