import { FrameSocket, type SocketState } from './socket'
import type { Frame } from './framer'
import { inDesktopApp, startNativeCapture } from './native'
import workletUrl from './worklet.ts?worker&url'

/** "tab": the shared meeting tab's audio plus the microphone. "room": the microphone only. */
export type CaptureMode = 'tab' | 'room'
export type CaptureState = 'recording' | 'reconnecting' | 'stopped'

export const NO_TAB_AUDIO = 'Share a browser tab with "Share tab audio" turned on. A window, a screen or a tab without its audio cannot be recorded.'

/** A refusal or a failure before recording starts; `message` is meant for the member. */
export class CaptureError extends Error {
  /** Desktop app only: the missing permission ("microphone" or "audio_capture") whose System Settings pane can be opened. */
  constructor(message: string, readonly pane?: string) { super(message) }
}

export type Capture = {
  readonly mode: CaptureMode
  /** True when the desktop app captures the Mac's audio; `levels.mic` is then the mixed level. */
  readonly native?: boolean
  state: CaptureState
  /** Latest peak per input, 0 to 1: the microphone, and the tab in "tab" mode. */
  levels: { mic: number; tab?: number }
  /** True once the shared tab (or, in the desktop app, the system audio) stopped sending audio (the member ended the share or closed the tab). */
  tabEnded: boolean
  /** Frames lost because the network was down for over 60 s. */
  dropped: number
  /** Called whenever state, levels, tabEnded or dropped change. */
  onChange?: () => void
  stop(): void
  discard(): void
}

/** Starts recording and streams 16 kHz frames to `socketUrl` (a path on this origin, or a ws(s) URL). */
export async function startCapture(mode: CaptureMode, socketUrl: string): Promise<Capture> {
  if (inDesktopApp()) return startNativeCapture(mode, wsUrl(socketUrl))
  const streams: MediaStream[] = []
  const release = () => streams.forEach((s) => s.getTracks().forEach((t) => t.stop()))
  let tab: MediaStream | undefined
  try {
    if (mode === 'tab') {
      tab = await ask(() => navigator.mediaDevices.getDisplayMedia({ video: true, audio: true }))
      streams.push(tab)
      const surface = tab.getVideoTracks()[0]?.getSettings().displaySurface
      if (tab.getAudioTracks().length === 0 || surface !== 'browser') throw new CaptureError(NO_TAB_AUDIO)
    }
    const mic = await ask(() =>
      navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } }),
    )
    streams.push(mic)
    return await wire(mode, socketUrl, mic, tab, release)
  } catch (e) {
    release()
    throw e
  }
}

async function wire(mode: CaptureMode, socketUrl: string, mic: MediaStream, tab: MediaStream | undefined, release: () => void) {
  const ctx = new AudioContext()
  let node: AudioWorkletNode
  try {
    await ctx.audioWorklet.addModule(workletUrl)
    node = new AudioWorkletNode(ctx, 'recapz-pcm', {
      numberOfInputs: 2,
      numberOfOutputs: 0,
      channelCount: 1,
      channelCountMode: 'explicit',
    })
    ctx.createMediaStreamSource(mic).connect(node, 0, 0)
    if (tab) ctx.createMediaStreamSource(new MediaStream(tab.getAudioTracks())).connect(node, 0, 1)
    // Created after awaited prompts, so it may start suspended.
    await ctx.resume()
  } catch (e) {
    void ctx.close()
    throw e
  }

  const capture: Capture = {
    mode,
    state: 'recording',
    levels: tab ? { mic: 0, tab: 0 } : { mic: 0 },
    tabEnded: false,
    dropped: 0,
    stop: () => finish(() => socket.stop()),
    discard: () => finish(() => socket.discard()),
  }
  const changed = () => capture.onChange?.()
  const socket = new FrameSocket(wsUrl(socketUrl), (s: SocketState) => {
    if (capture.state === 'stopped') return
    if (s === 'closed') return finish(() => {}) // the socket gave up: the server has ended this recording
    capture.state = s === 'reconnecting' ? 'reconnecting' : 'recording'
    changed()
  })
  const finish = (end: () => void) => {
    if (capture.state === 'stopped') return
    capture.state = 'stopped'
    node.port.onmessage = null
    end()
    release()
    void ctx.close()
    changed()
  }

  node.port.onmessage = (e: MessageEvent<Frame>) => {
    socket.send(e.data.pcm)
    const [m = 0, t = 0] = e.data.levels
    capture.levels = tab ? { mic: m, tab: t } : { mic: m }
    capture.dropped = socket.dropped
    changed()
  }
  tab?.getAudioTracks()[0].addEventListener('ended', () => {
    capture.tabEnded = true
    changed()
  })
  return capture
}

/** Runs a media prompt, turning a refusal into a message for the member. */
async function ask(prompt: () => Promise<MediaStream>) {
  try {
    return await prompt()
  } catch (e) {
    const name = e instanceof DOMException ? e.name : ''
    if (name === 'NotAllowedError') throw new CaptureError('Recording needs permission to use the microphone and the shared tab.')
    if (name === 'NotFoundError') throw new CaptureError('No microphone was found.')
    throw new CaptureError('This browser cannot record. Use desktop Chrome or Edge, or upload the recording instead.')
  }
}

function wsUrl(path: string) {
  const u = new URL(path, location.href)
  if (u.protocol === 'http:') u.protocol = 'ws:'
  if (u.protocol === 'https:') u.protocol = 'wss:'
  return u.href
}
