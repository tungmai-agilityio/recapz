import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { FrameSocket, MAX_BUFFERED } from './socket'

class FakeSocket {
  static OPEN = 1
  static all: FakeSocket[] = []
  readyState = 0
  binaryType = ''
  sent: (ArrayBuffer | string)[] = []
  closed = false
  onopen: (() => void) | null = null
  onclose: (() => void) | null = null
  url: string
  constructor(url: string) {
    this.url = url
    FakeSocket.all.push(this)
  }
  send(d: ArrayBuffer | string) {
    this.sent.push(d)
  }
  close() {
    this.closed = true
  }
  open() {
    this.readyState = 1
    this.onopen?.()
  }
  drop() {
    this.readyState = 3
    this.onclose?.()
  }
}

const frame = (n: number) => new Uint8Array([n % 256, n >> 8]).buffer
const ids = (sent: (ArrayBuffer | string)[]) => sent.map((d) => (typeof d === 'string' ? d : new Uint8Array(d)[0] + (new Uint8Array(d)[1] << 8)))
const last = () => FakeSocket.all[FakeSocket.all.length - 1]

beforeEach(() => {
  FakeSocket.all = []
  vi.useFakeTimers()
  vi.stubGlobal('WebSocket', FakeSocket)
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

test('sends frames as they come while open, as array buffers', () => {
  const s = new FrameSocket('ws://x/rec')
  expect(last().binaryType).toBe('arraybuffer')
  s.send(frame(1))
  last().open()
  s.send(frame(2))
  expect(ids(last().sent)).toEqual([1, 2])
})

test('buffers while disconnected, reconnects with backoff and resends in order', () => {
  const states: string[] = []
  const s = new FrameSocket('ws://x/rec', (st) => states.push(st))
  last().open()
  s.send(frame(1))
  last().drop()
  for (let i = 2; i <= 5; i++) s.send(frame(i))
  expect(FakeSocket.all).toHaveLength(1)
  vi.advanceTimersByTime(499)
  expect(FakeSocket.all).toHaveLength(1)
  vi.advanceTimersByTime(1)
  expect(FakeSocket.all).toHaveLength(2)
  last().drop() // a failed attempt waits longer
  vi.advanceTimersByTime(999)
  expect(FakeSocket.all).toHaveLength(2)
  vi.advanceTimersByTime(1)
  expect(FakeSocket.all).toHaveLength(3)
  s.send(frame(6))
  last().open()
  s.send(frame(7))
  expect(ids(FakeSocket.all[0].sent)).toEqual([1])
  expect(ids(last().sent)).toEqual([2, 3, 4, 5, 6, 7])
  expect(states).toEqual(['open', 'reconnecting', 'open'])
})

test('keeps at most 60 s of frames, dropping the oldest', () => {
  const s = new FrameSocket('ws://x/rec')
  for (let i = 0; i < MAX_BUFFERED + 5; i++) s.send(frame(i))
  expect(s.dropped).toBe(5)
  last().open()
  const got = ids(last().sent)
  expect(got).toHaveLength(600)
  expect(got[0]).toBe(5)
  expect(got[599]).toBe(604)
})

test('stop is sent after the buffered frames, then the socket closes and does not reconnect', () => {
  const s = new FrameSocket('ws://x/rec')
  s.send(frame(1))
  s.stop()
  s.send(frame(2))
  expect(last().sent).toEqual([])
  last().open()
  expect(ids(last().sent)).toEqual([1, 'stop'])
  expect(last().closed).toBe(true)
  expect(s.state).toBe('closed')
  last().drop()
  vi.advanceTimersByTime(60_000)
  expect(FakeSocket.all).toHaveLength(1)
})

test('stop while disconnected is delivered after reconnecting', () => {
  const s = new FrameSocket('ws://x/rec')
  last().open()
  last().drop()
  s.send(frame(9))
  s.stop()
  vi.advanceTimersByTime(500)
  last().open()
  expect(ids(last().sent)).toEqual([9, 'stop'])
})

test('discard drops buffered frames and sends only discard', () => {
  const s = new FrameSocket('ws://x/rec')
  s.send(frame(1))
  s.send(frame(2))
  s.discard()
  last().open()
  expect(last().sent).toEqual(['discard'])
  expect(last().closed).toBe(true)
})
