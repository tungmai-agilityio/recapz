/** 60 s of 100 ms frames. */
export const MAX_BUFFERED = 600
const BACKOFF_MS = [500, 1000, 2000, 4000, 8000]
/** The server ends a call after 60 s without frames, so a socket down this long has nothing left to reach. */
const GIVE_UP_MS = 60_000

export type SocketState = 'connecting' | 'open' | 'reconnecting' | 'closed'

/**
 * Sends frames and the `stop` / `discard` commands in order over a WebSocket. While the socket is down it keeps
 * up to 60 s of frames, reconnects with backoff and resends them first. It gives up (`closed`) once the server can no
 * longer take the recording: down for over 60 s, or a `stop` / `discard` still undelivered after every backoff step.
 */
export class FrameSocket {
  state: SocketState = 'connecting'
  /** Frames dropped because the buffer was full (the oldest go first). */
  dropped = 0
  private ws!: WebSocket
  private queue: (ArrayBuffer | string)[] = []
  private attempt = 0
  private downSince = Date.now()
  private ending = false
  private timer: ReturnType<typeof setTimeout> | undefined
  private readonly url: string
  private readonly onState: (s: SocketState) => void

  constructor(url: string, onState: (s: SocketState) => void = () => {}) {
    this.url = url
    this.onState = onState
    this.connect()
  }

  send(frame: ArrayBuffer) {
    if (this.ending) return
    this.queue.push(frame)
    if (this.queue.length > MAX_BUFFERED) {
      this.queue.shift()
      this.dropped++
    }
    this.flush()
  }

  /** Ends the recording; the server transcribes it. */
  stop() {
    this.end('stop')
  }

  /** Ends the recording and asks the server to delete it; buffered frames are dropped. */
  discard() {
    this.queue = []
    this.end('discard')
  }

  private end(command: string) {
    if (this.ending) return
    this.ending = true
    this.queue.push(command)
    this.flush()
  }

  private connect() {
    const ws = new WebSocket(this.url)
    ws.binaryType = 'arraybuffer'
    ws.onopen = () => {
      this.attempt = 0
      this.set('open')
      this.flush()
    }
    ws.onclose = () => {
      if (this.state === 'closed') return
      if (this.ending && this.queue.length === 0) return this.set('closed')
      if (this.state === 'open') this.downSince = Date.now()
      if (Date.now() - this.downSince > GIVE_UP_MS || (this.ending && this.attempt >= BACKOFF_MS.length)) {
        this.queue = []
        return this.set('closed')
      }
      this.set('reconnecting')
      const delay = BACKOFF_MS[Math.min(this.attempt++, BACKOFF_MS.length - 1)]
      this.timer = setTimeout(() => this.connect(), delay)
    }
    this.ws = ws
  }

  private flush() {
    if (this.ws.readyState !== WebSocket.OPEN) return
    while (this.queue.length) {
      const next = this.queue.shift()!
      this.ws.send(next)
      if (typeof next === 'string') {
        clearTimeout(this.timer)
        this.set('closed')
        this.ws.close()
        return
      }
    }
  }

  private set(s: SocketState) {
    if (this.state === s) return
    this.state = s
    this.onState(s)
  }
}
