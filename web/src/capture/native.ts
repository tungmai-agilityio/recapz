import { CaptureError, type Capture, type CaptureMode } from './capture'
import { FrameSocket, type SocketState } from './socket'

type Tauri = {
  core: {
    invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>
    Channel: new <T>() => { onmessage: (m: T) => void }
  }
}
declare global {
  interface Window { __TAURI__?: Tauri }
}

/** True inside the Recapz desktop app, which records through the Mac's audio instead of the browser. */
export const inDesktopApp = () => !!window.__TAURI__

const SYSTEM_STOPPED = 'recapz-system-audio-stopped'

const MISSING: Record<string, string> = {
  microphone: 'Microphone',
  audio_capture: 'System Audio Recording',
}

/** Starts the app's native capture and streams its 16 kHz frames to `socketUrl` (a ws(s) URL). */
export async function startNativeCapture(mode: CaptureMode, wsUrl: string): Promise<Capture> {
  const { invoke, Channel } = window.__TAURI__!.core
  const native = mode === 'tab' ? 'meeting' : 'in_room'
  const granted = String(await invoke('audio_permission', { mode: native }))
  if (granted !== 'granted') {
    const names = granted.split(',').map((m) => MISSING[m] ?? m).join(' and ')
    throw new CaptureError(`Recapz needs permission for ${names}. Open System Settings, Privacy & Security, turn on Recapz there, then try again.`)
  }

  const capture: Capture = {
    mode, native: true, state: 'recording', levels: { mic: 0 }, tabEnded: false, dropped: 0,
    stop: () => finish(() => socket.stop()),
    discard: () => finish(() => socket.discard()),
  }
  const socket = new FrameSocket(wsUrl, (s: SocketState) => {
    if (capture.state === 'stopped') return
    if (s === 'closed') return finish(() => {}) // the server has ended this recording
    capture.state = s === 'reconnecting' ? 'reconnecting' : 'recording'
    capture.onChange?.()
  })
  // The shell fires this when the system-audio tap stops or fails; the microphone keeps going.
  const onSystemStopped = () => {
    capture.tabEnded = true
    capture.onChange?.()
  }
  const frames = new Channel<ArrayBuffer>()
  const finish = (end: () => void) => {
    if (capture.state === 'stopped') return
    capture.state = 'stopped'
    frames.onmessage = () => {}
    window.removeEventListener(SYSTEM_STOPPED, onSystemStopped)
    end()
    void invoke('recording_state', { on: false })
    void invoke('audio_stop')
    capture.onChange?.()
  }
  frames.onmessage = (pcm) => {
    socket.send(pcm)
    let peak = 0
    for (const s of new Int16Array(pcm)) peak = Math.max(peak, Math.abs(s))
    capture.levels = { mic: peak / 32768 }
    capture.dropped = socket.dropped
    capture.onChange?.()
  }
  try {
    await invoke('audio_start', { frames, mode: native })
  } catch (e) {
    socket.discard()
    throw new CaptureError(typeof e === 'string' ? e : 'The app could not start recording.')
  }
  void invoke('recording_state', { on: true })
  if (mode === 'tab') window.addEventListener(SYSTEM_STOPPED, onSystemStopped)
  return capture
}
