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
      sent: unknown[] = []
      readyState = 1
      binaryType = ''
      constructor(public url: string) { sockets.push(this) }
      send(d: unknown) { this.sent.push(d) }
      close() {}
    },
    { OPEN: 1 },
  ))
  vi.stubGlobal('window', {
    __TAURI__: {
      core: {
        invoke,
        Channel: class { onmessage = () => {}; constructor() { channel = this as never } },
      },
    },
  })
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
