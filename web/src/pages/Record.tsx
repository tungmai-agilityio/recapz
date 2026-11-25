import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useReducer, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router'
import { startRecording, type MeetingLanguage } from '../api'
import { CaptureError, startCapture, type Capture, type CaptureMode } from '../capture/capture'
import { inDesktopApp, openPrivacySettings } from '../capture/native'
import { FrameSocket } from '../capture/socket'
import { clock } from '../clock'
import { Button } from '../ui/Button'
import { Choice } from '../ui/Choice'
import { ConfirmDialog } from '../ui/ConfirmDialog'
import { Field, inputClass } from '../ui/Field'
import { useToast } from '../ui/toast-context'

/** The recording this tab is making. It lives outside React so it survives moving between pages, and only this tab has it. */
let active: { id: string; capture: Capture; since: number } | null = null

const socketPath = (id: string) => `/api/meetings/${id}/record`

/** Desktop Chrome and Edge share a tab's audio; `userAgentData` exists only in Chromium browsers. */
function canRecordTab() {
  const ua = (navigator as Navigator & { userAgentData?: { mobile: boolean } }).userAgentData
  return !!ua && !ua.mobile && typeof navigator.mediaDevices?.getDisplayMedia === 'function'
}

const APP_MODES = [
  { value: 'tab', label: 'Meeting app', hint: 'The call in Zoom, Teams, Meet or any app on this Mac, plus your microphone. Use headphones, and turn off other music and videos: everything the Mac plays is recorded.' },
  { value: 'room', label: 'In-room', hint: 'Your microphone only, for people in the same room.' },
]

const MODES = [
  { value: 'tab', label: 'Meeting tab', hint: 'The call in another tab of this browser, plus your microphone. Share the tab with "Share tab audio" on.' },
  { value: 'room', label: 'In-room', hint: 'Your microphone only, for people in the same room.' },
]

export function RecordDialog({ onClose }: { onClose: () => void }) {
  const toast = useToast()
  const navigate = useNavigate()
  const qc = useQueryClient()
  const [title, setTitle] = useState('')
  const [language, setLanguage] = useState<MeetingLanguage>('vi')
  const [mode, setMode] = useState<CaptureMode>('tab')
  const [told, setTold] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [pane, setPane] = useState<string>()
  const desktop = inDesktopApp()
  const supported = desktop || canRecordTab()
  const notice = `This meeting is recorded with Recapz. The audio is transcribed to write the meeting report.`

  // While starting, closing would leave start() running on an unmounted dialog.
  const close = useRef(onClose)
  useEffect(() => { close.current = busy ? () => {} : onClose })
  useEffect(() => {
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') close.current() }
    document.addEventListener('keydown', esc)
    return () => document.removeEventListener('keydown', esc)
  }, [])

  async function start() {
    setBusy(true)
    setError('')
    setPane(undefined)
    let id = ''
    try {
      id = (await startRecording(title.trim(), language, mode)).id
      const capture = await startCapture(mode, socketPath(id))
      active = { id, capture, since: Date.now() }
      void qc.invalidateQueries({ queryKey: ['meetings'] })
      navigate(`/meetings/${id}/live`)
    } catch (e) {
      // The meeting exists and is recording: discard it so no empty meeting is left behind.
      if (id) new FrameSocket(new URL(socketPath(id), location.href).href.replace(/^http/, 'ws')).discard()
      setError(e instanceof CaptureError ? e.message : 'The recording could not start. Try again.')
      setPane(e instanceof CaptureError ? e.pane : undefined)
      setBusy(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-ink/40 p-4">
      <form role="dialog" aria-modal="true" aria-label="Record from this browser"
        className="flex max-h-full w-full max-w-md flex-col gap-4 overflow-y-auto rounded-lg border border-hair bg-sheet p-5 shadow-xl"
        onSubmit={(e) => { e.preventDefault(); void start() }}>
        <h2 className="text-lg font-semibold">Record from this browser</h2>
        {!supported ? (
          <p role="alert" className="text-sm text-ink-2">
            This browser cannot record a meeting tab. Use desktop Chrome or Edge, or <Link to="/upload" className="text-primary underline">upload the recording</Link> after the call.
          </p>
        ) : (
          <>
            <Field label="Title">
              {(p) => <input {...p} required autoFocus maxLength={200} value={title} onChange={(e) => setTitle(e.target.value)} className={inputClass} />}
            </Field>
            <Field label="Language" hint="Mixed is for calls that switch between Vietnamese and English.">
              {(p) => (
                <select {...p} className={inputClass} value={language} onChange={(e) => setLanguage(e.target.value as MeetingLanguage)}>
                  <option value="vi">Vietnamese</option>
                  <option value="en">English</option>
                  <option value="multi">Mixed languages</option>
                </select>
              )}
            </Field>
            <Choice label="What to record" options={desktop ? APP_MODES : MODES} value={mode} onChange={(v) => setMode(v as CaptureMode)} />
            <label className="grid grid-cols-[18px_1fr] gap-2.5 text-sm">
              <input type="checkbox" checked={told} onChange={(e) => setTold(e.target.checked)} className="mt-1 accent-primary" />
              <span>I have told everyone in this meeting that it is recorded.</span>
            </label>
            <div className="flex flex-col gap-1.5 rounded-lg border border-hair bg-inset p-3 text-sm">
              <span className="text-ink-2">{notice}</span>
              <Button className="self-start" onClick={() => navigator.clipboard.writeText(notice).then(() => toast('Copied. Paste it in the call chat.'), () => toast('Could not copy. Select the text instead.'))}>
                Copy for the call chat
              </Button>
            </div>
            {error && <p role="alert" className="text-sm text-bad">{error}</p>}
            {pane && <Button className="self-start" onClick={() => void openPrivacySettings(pane)}>Open System Settings</Button>}
          </>
        )}
        <div className="flex justify-end gap-2">
          <Button onClick={onClose} disabled={busy}>Cancel</Button>
          {supported && (
            <Button variant="primary" type="submit" disabled={!told || !title.trim() || busy}>{busy ? 'Starting…' : 'Start recording'}</Button>
          )}
        </div>
      </form>
    </div>
  )
}

/** Elapsed time, input levels, Stop and Stop and discard. Shown only in the tab that is recording this meeting. */
export function RecorderBar({ id, ended }: { id: string; ended: boolean }) {
  const [, render] = useReducer((n: number) => n + 1, 0)
  const [discarding, setDiscarding] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const toast = useToast()
  const navigate = useNavigate()
  const rec = active?.id === id ? active : null
  const capture = rec?.capture
  const finish = (end: () => void) => {
    end()
    active = null
    render()
  }

  useEffect(() => {
    if (!capture) return
    capture.onChange = render
    const tick = setInterval(() => setNow(Date.now()), 1000)
    const leave = (e: BeforeUnloadEvent) => { e.preventDefault() }
    window.addEventListener('beforeunload', leave)
    return () => { capture.onChange = undefined; clearInterval(tick); window.removeEventListener('beforeunload', leave) }
  }, [capture])
  // The server ended the call (time limit, or no audio for 60 s): stop capturing too.
  useEffect(() => {
    if (ended && capture && active?.capture === capture) { capture.stop(); active = null; render() }
  }, [ended, capture])

  if (!rec || !capture || capture.state === 'stopped') return null
  const meters: [string, number | undefined][] = capture.native ? [['Audio', capture.levels.mic]] : [['Microphone', capture.levels.mic], ['Meeting tab', capture.levels.tab]]
  return (
    <div role="region" aria-label="Recorder" className="frame flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-4">
        <span className="font-mono text-sm" aria-label="Elapsed time">{clock(Math.max(0, now - rec.since))}</span>
        {meters.filter(([, v]) => v !== undefined).map(([label, v]) => (
          <label key={label} className="flex items-center gap-2 text-xs text-mute">
            {label}<meter min={0} max={1} value={v} className="w-24" />
          </label>
        ))}
        <span className="ml-auto flex gap-2">
          <Button variant="primary" onClick={() => finish(() => capture.stop())}>Stop</Button>
          <Button variant="danger" onClick={() => setDiscarding(true)}>Stop and discard</Button>
        </span>
      </div>
      {capture.state === 'reconnecting' && <p role="status" className="text-sm text-mute">Connection lost. Reconnecting; up to 60 s of audio is kept meanwhile.</p>}
      {capture.dropped > 0 && <p role="alert" className="text-sm text-bad">The connection was down for over 60 s, so {(capture.dropped / 10).toFixed(0)} s of audio was lost.</p>}
      {capture.tabEnded && <p role="alert" className="text-sm text-bad">{capture.native ? 'System audio stopped. Only your microphone is recorded now.' : 'The meeting tab stopped sharing its audio. Only your microphone is recorded now.'}</p>}
      {discarding && (
        <ConfirmDialog title="Stop and discard?" confirmLabel="Discard recording" onCancel={() => setDiscarding(false)}
          onConfirm={() => { finish(() => capture.discard()); toast('Recording discarded'); navigate('/') }}>
          The recording stops and its audio and live transcript are deleted permanently. This cannot be undone.
        </ConfirmDialog>
      )}
    </div>
  )
}
