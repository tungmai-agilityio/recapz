import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { startCapture } from './capture'

let sockets: { url: string; sent: unknown[] }[]
let channel: { onmessage: (m: ArrayBuffer) => void }
let invoke: ReturnType<typeof vi.fn>
let permission: string

beforeEach(() => {
  sockets = []
  permission = 'granted'
  invoke = vi.fn(async (cmd: string) => (cmd === 'audio_permission' ? permission : undefined))
  vi.stubGlobal('WebSocket', Object.assign(
    class {
      url: string
      sent: unknown[] = []
      readyState = 1
      binaryType = ''
      constructor(url: string) { this.url = url; sockets.push(this) }
      send(d: unknown) { this.sent.push(d) }
      close() {}
    },
    { OPEN: 1 },
  ))
  vi.stubGlobal('window', Object.assign(new EventTarget(), {
    __TAURI__: {
      core: {
        invoke,
        Channel: class { onmessage = () => {}; constructor() { channel = this as never } },
      },
    },
  }))
  vi.stubGlobal('location', { href: 'http://localhost/' })
})
afterEach(() => vi.unstubAllGlobals())

test('meeting mode starts the native capture and sends its frames to the socket', async () => {
  const c = await startCapture('tab', '/api/meetings/m1/record')
  expect(invoke).toHaveBeenCalledWith('audio_permission', { mode: 'meeting' })
  expect(invoke).toHaveBeenCalledWith('audio_start', { frames: channel, mode: 'meeting' })
  expect(c.native).toBe(true)
  const pcm = new Int16Array([0, 16384, -8192]).buffer
  channel.onmessage(pcm)
  expect(sockets[0].url).toBe('ws://localhost/api/meetings/m1/record')
  expect(sockets[0].sent).toEqual([pcm])
  expect(c.levels.mic).toBe(0.5)
})

test('in-room mode asks only for the in-room capture; stop ends the socket and the native capture', async () => {
  const c = await startCapture('room', '/api/meetings/m1/record')
  expect(invoke).toHaveBeenCalledWith('audio_start', { frames: channel, mode: 'in_room' })
  c.stop()
  expect(sockets[0].sent).toEqual(['stop'])
  expect(invoke).toHaveBeenLastCalledWith('audio_stop')
  expect(c.state).toBe('stopped')
})

test('a missing permission names what to grant and does not start', async () => {
  permission = 'microphone,audio_capture'
  await expect(startCapture('tab', '/x')).rejects.toThrow(/Microphone and System Audio Recording/)
  expect(invoke).not.toHaveBeenCalledWith('audio_start', expect.anything())
  expect(sockets).toHaveLength(0)
})

test('the menu-bar dot goes on when a recording starts and off when it ends', async () => {
  const c = await startCapture('room', '/x')
  expect(invoke).toHaveBeenCalledWith('recording_state', { on: true })
  c.discard()
  expect(invoke).toHaveBeenCalledWith('recording_state', { on: false })
  expect(invoke.mock.calls.filter(([cmd]) => cmd === 'recording_state')).toHaveLength(2)
})

test('the dot never goes on when the capture does not start', async () => {
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'audio_permission') return 'granted'
    if (cmd === 'audio_start') throw 'no device'
  })
  await expect(startCapture('room', '/x')).rejects.toThrow('no device')
  expect(invoke).not.toHaveBeenCalledWith('recording_state', { on: true })
})

test('a system-audio-stopped event from the shell sets tabEnded and keeps recording', async () => {
  const c = await startCapture('tab', '/x')
  const changed = vi.fn()
  c.onChange = changed
  expect(c.tabEnded).toBe(false)
  window.dispatchEvent(new Event('recapz-system-audio-stopped'))
  expect(c.tabEnded).toBe(true)
  expect(c.state).toBe('recording')
  expect(changed).toHaveBeenCalledTimes(1)
  c.stop()
  window.dispatchEvent(new Event('recapz-system-audio-stopped'))
  expect(changed).toHaveBeenCalledTimes(2) // only stop's own change, not the late event
})

test('in-room mode ignores the system-audio event', async () => {
  const c = await startCapture('room', '/x')
  window.dispatchEvent(new Event('recapz-system-audio-stopped'))
  expect(c.tabEnded).toBe(false)
})
