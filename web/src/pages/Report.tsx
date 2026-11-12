import { useQuery } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { Link, useParams, useSearchParams } from 'react-router'
import { clock } from '../clock'
import { getMeeting, getTranscript, Unauthorized } from '../api'

export function Report() {
  const { id = '' } = useParams()
  const [params] = useSearchParams()
  const at = Number(params.get('t')) // ms, set by Ask citation cards
  const meeting = useQuery({ queryKey: ['meeting', id], queryFn: () => getMeeting(id) })
  const audio = useRef<HTMLAudioElement>(null)
  const [now, setNow] = useState(params.has('t') && Number.isFinite(at) ? at : 0) // playhead, ms
  const transcript = useQuery({
    queryKey: ['transcript', id],
    queryFn: () => getTranscript(id),
    enabled: meeting.isSuccess,
  })

  // Open at the cited moment once the transcript is on screen.
  useEffect(() => {
    if (!transcript.data || !params.has('t') || !Number.isFinite(at)) return
    if (audio.current) audio.current.currentTime = at / 1000
    document.querySelector('[aria-current]')?.scrollIntoView?.({ block: 'center' })
  }, [transcript.data, at, params])

  if (meeting.error instanceof Unauthorized) return <Link to="/" className="text-primary">Sign in</Link>
  if (meeting.isError) return <p className="text-mute">Meeting not found.</p>
  if (!meeting.data) return <p className="text-mute">Loading…</p>
  const m = meeting.data

  return (
    <article>
      <Link to="/" className="text-sm text-mute hover:text-primary">← Meetings</Link>
      <h1 className="mt-2 text-2xl font-semibold tracking-tight">{m.title}</h1>
      <p className="font-mono text-xs text-mute">{m.platform} · {m.status}</p>
      {m.error && <p className="mt-3 text-sm text-red-700">{m.error}</p>}

      <audio
        ref={audio}
        controls
        preload="metadata"
        src={`/api/meetings/${id}/audio`}
        onTimeUpdate={(e) => setNow(e.currentTarget.currentTime * 1000)}
        className="mt-4 w-full"
      />

      <h2 className="mt-8 text-lg font-semibold">Transcript</h2>
      {transcript.isLoading && <p className="mt-3 text-mute">Loading…</p>}
      {transcript.isError && <p className="mt-3 text-red-700">Could not load the transcript.</p>}
      {transcript.data?.length === 0 && (
        <p className="mt-3 text-mute">
          {m.status === 'ready' ? 'No speech was found.' : 'The transcript will appear when processing finishes.'}
        </p>
      )}
      <ol className="mt-3 space-y-3">
        {transcript.data?.map((s) => {
          const active = now >= s.start_ms && now < s.end_ms
          return (
            <li key={s.id}>
              <button
                type="button"
                aria-current={active || undefined}
                onClick={() => {
                  if (!audio.current) return
                  audio.current.currentTime = s.start_ms / 1000
                  setNow(s.start_ms)
                  void audio.current.play().catch(() => {})
                }}
                className={`flex w-full gap-4 rounded-md border-l-2 px-2 py-1 text-left hover:bg-sheet ${active ? 'border-signal bg-sheet' : 'border-transparent'}`}
              >
                <time className="w-14 shrink-0 pt-0.5 font-mono text-xs text-mute">{clock(s.start_ms)}</time>
                <span>{s.text}</span>
              </button>
            </li>
          )
        })}
      </ol>
    </article>
  )
}
