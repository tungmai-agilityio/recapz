import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { NO_TAB_AUDIO, startCapture } from './capture'

class FakeTrack extends EventTarget {
  stopped = false
  kind: string
  surface: string
  constructor(kind: string, surface = 'browser') {
    super()
    this.kind = kind
    this.surface = surface
  }
  stop() {
    this.stopped = true
  }
  getSettings() {
    return { displaySurface: this.surface }
  }
}

class FakeStream {
  tracks: FakeTrack[]
  constructor(tracks: FakeTrack[] = []) {
    this.tracks = tracks
  }
  getTracks() {
    return this.tracks
  }
  getAudioTracks() {
    return this.tracks.filter((t) => t.kind === 'audio')
  }
  getVideoTracks() {
    return this.tracks.filter((t) => t.kind === 'video')
  }
}

type Link = { from: FakeStream; input: number }
let links: Link[]
let node: { port: { onmessage: ((e: { data: unknown }) => void) | null }; options: unknown }
let sockets: { url: string; sent: unknown[]; readyState: number; onopen?: () => void }[]
let ctxClosed: boolean
let display: ReturnType<typeof vi.fn>
let user: ReturnType<typeof vi.fn>
let addModule: ReturnType<typeof vi.fn>

beforeEach(() => {
  links = []
  sockets = []
  ctxClosed = false
  addModule = vi.fn(async () => {})
  vi.stubGlobal('MediaStream', FakeStream)
  vi.stubGlobal(
    'AudioContext',
    class {
      audioWorklet = { addModule: addModule }
      async resume() {}
      createMediaStreamSource(from: FakeStream) {
        return { connect: (_n: unknown, _o: number, input: number) => links.push({ from, input }) }
      }
      async close() {
        ctxClosed = true
      }
    },
  )
  vi.stubGlobal(
    'AudioWorkletNode',
    class {
      port = { onmessage: null }
      options: unknown
      constructor(_ctx: unknown, _name: string, options: unknown) {
        this.options = options
        node = this as never
      }
    },
  )
  vi.stubGlobal(
    'WebSocket',
    Object.assign(
      class {
        url: string
        sent: unknown[] = []
        readyState = 1
        binaryType = ''
        constructor(url: string) {
          this.url = url
          sockets.push(this as never)
        }
        send(d: unknown) {
          this.sent.push(d)
        }
        close() {}
      },
      { OPEN: 1 },
    ),
  )
  display = vi.fn()
  user = vi.fn(async () => new FakeStream([new FakeTrack('audio')]))
  vi.stubGlobal('navigator', { mediaDevices: { getDisplayMedia: display, getUserMedia: user } })
})
afterEach(() => vi.unstubAllGlobals())

test('tab mode mixes tab audio with the echo-cancelled microphone and streams frames', async () => {
  const tabAudio = new FakeTrack('audio')
  const tab = new FakeStream([new FakeTrack('video'), tabAudio])
  display.mockResolvedValue(tab)
  const c = await startCapture('tab', '/api/meetings/m1/record')

  expect(display).toHaveBeenCalledWith({ video: true, audio: true })
  expect(user.mock.calls[0][0].audio.echoCancellation).toBe(true)
  expect(node.options).toMatchObject({ numberOfInputs: 2, channelCount: 1, channelCountMode: 'explicit' })
  expect(links).toHaveLength(2)
  expect(links.map((l) => [l.input, l.from.getAudioTracks()[0] === tabAudio])).toEqual([[0, false], [1, true]])
  expect(sockets[0].url).toBe(`ws://${location.host}/api/meetings/m1/record`)

  const pcm = new ArrayBuffer(3200)
  node.port.onmessage!({ data: { pcm, levels: [0.2, 0.7] } })
  expect(sockets[0].sent).toEqual([pcm])
  expect(c.levels).toEqual({ mic: 0.2, tab: 0.7 })
  expect(c.state).toBe('recording')

  tabAudio.dispatchEvent(new Event('ended'))
  expect(c.tabEnded).toBe(true)
})

test('room mode records only the microphone', async () => {
  const c = await startCapture('room', 'ws://x/rec')
  expect(display).not.toHaveBeenCalled()
  expect(links).toHaveLength(1)
  expect(links[0].input).toBe(0)
  node.port.onmessage!({ data: { pcm: new ArrayBuffer(3200), levels: [0.4] } })
  expect(c.levels).toEqual({ mic: 0.4 })
})

test.each([
  ['a tab without its audio', [new FakeTrack('video')]],
  ['a window with audio', [new FakeTrack('video', 'window'), new FakeTrack('audio')]],
  ['a screen with audio', [new FakeTrack('video', 'monitor'), new FakeTrack('audio')]],
])('refuses %s and releases the share', async (_name, tracks) => {
  display.mockResolvedValue(new FakeStream(tracks))
  await expect(startCapture('tab', 'ws://x/rec')).rejects.toThrow(NO_TAB_AUDIO)
  expect(tracks.every((t) => t.stopped)).toBe(true)
  expect(user).not.toHaveBeenCalled()
  expect(sockets).toHaveLength(0)
})

test('a denied microphone fails with a message and releases the tab', async () => {
  const tracks = [new FakeTrack('video'), new FakeTrack('audio')]
  display.mockResolvedValue(new FakeStream(tracks))
  user.mockRejectedValue(new DOMException('no', 'NotAllowedError'))
  await expect(startCapture('tab', 'ws://x/rec')).rejects.toThrow('Recording needs permission')
  expect(tracks.every((t) => t.stopped)).toBe(true)
})

test('stop sends stop, releases the devices and ignores later frames', async () => {
  const mic = new FakeStream([new FakeTrack('audio')])
  user.mockResolvedValue(mic)
  const c = await startCapture('room', 'ws://x/rec')
  let changes = 0
  c.onChange = () => changes++
  c.stop()
  expect(sockets[0].sent).toEqual(['stop'])
  expect(mic.tracks[0].stopped).toBe(true)
  expect(ctxClosed).toBe(true)
  expect(c.state).toBe('stopped')
  expect(node.port.onmessage).toBeNull()
  expect(changes).toBe(1)
  c.discard()
  expect(sockets[0].sent).toEqual(['stop'])
})

test('discard sends discard', async () => {
  const c = await startCapture('room', 'ws://x/rec')
  c.discard()
  expect(sockets[0].sent).toEqual(['discard'])
})

test('a failed audio setup closes the context and releases the microphone', async () => {
  const mic = new FakeStream([new FakeTrack('audio')])
  user.mockResolvedValue(mic)
  addModule.mockRejectedValue(new Error('no worklet'))
  await expect(startCapture('room', 'ws://x/rec')).rejects.toThrow('no worklet')
  expect(ctxClosed).toBe(true)
  expect(mic.tracks[0].stopped).toBe(true)
  expect(sockets).toHaveLength(0)
})

test('a socket that gives up stops the capture and releases the devices', async () => {
  vi.useFakeTimers()
  const mic = new FakeStream([new FakeTrack('audio')])
  user.mockResolvedValue(mic)
  const c = await startCapture('room', 'ws://x/rec')
  const ws = sockets[0] as unknown as { onclose: () => void }
  vi.advanceTimersByTime(61_000)
  ws.onclose()
  expect(c.state).toBe('stopped')
  expect(mic.tracks[0].stopped).toBe(true)
  expect(ctxClosed).toBe(true)
  expect(sockets).toHaveLength(1)
  vi.useRealTimers()
})
