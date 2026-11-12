import { useQuery } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { Link, useParams, useSearchParams } from 'react-router'
import { clock } from '../clock'
import { getMeeting, getReport, getTranscript, Unauthorized, type ReportData } from '../api'

export function Report() {
  const { id = '' } = useParams()
  const [params] = useSearchParams()
  const at = Number(params.get('t')) // ms, set by Ask citation cards
  const meeting = useQuery({
    queryKey: ['meeting', id],
    queryFn: () => getMeeting(id),
    // The report is written in the background; keep checking until the meeting leaves `reporting`.
    refetchInterval: (q) => (q.state.data?.status === 'reporting' ? 3000 : false),
  })
  const audio = useRef<HTMLAudioElement>(null)
  const [now, setNow] = useState(params.has('t') && Number.isFinite(at) ? at : 0) // playhead, ms
  const transcript = useQuery({
    queryKey: ['transcript', id],
    queryFn: () => getTranscript(id),
    enabled: meeting.isSuccess,
  })

  const report = useQuery({
    queryKey: ['report', id],
    queryFn: () => getReport(id),
    enabled: meeting.isSuccess,
    refetchInterval: meeting.data?.status === 'reporting' ? 3000 : false,
  })
  const seek = (ms: number) => {
    if (!audio.current) return
    audio.current.currentTime = ms / 1000
    setNow(ms)
    void audio.current.play().catch(() => {})
  }
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
      {m.error && <p className="mt-3 text-sm text-bad">{m.error}</p>}

      <audio
        ref={audio}
        controls
        preload="metadata"
        src={`/api/meetings/${id}/audio`}
        onTimeUpdate={(e) => setNow(e.currentTarget.currentTime * 1000)}
        className="mt-4 w-full"
      />

      {report.data ? (
        <ReportSections r={report.data} seek={seek} />
      ) : (
        m.status === 'reporting' && <p className="mt-6 text-mute">The report will appear when processing finishes.</p>
      )}

      <h2 className="mt-8 text-lg font-semibold">Transcript</h2>
      {transcript.isLoading && <p className="mt-3 text-mute">Loading…</p>}
      {transcript.isError && <p className="mt-3 text-bad">Could not load the transcript.</p>}
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
                onClick={() => seek(s.start_ms)}
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

const Heading = ({ children }: { children: string }) => (
  <h2 className="mt-8 text-lg font-semibold">{children}</h2>
)

function ReportSections({ r, seek }: { r: ReportData; seek: (ms: number) => void }) {
  return (
    <>
      <Heading>Summary</Heading>
      <p className="mt-2 max-w-prose">{r.summary}</p>
      {r.decisions.length > 0 && (
        <>
          <Heading>Decisions</Heading>
          <ul className="mt-2 list-disc space-y-1 pl-5">{r.decisions.map((d, i) => <li key={i}>{d}</li>)}</ul>
        </>
      )}
      {r.open_questions.length > 0 && (
        <>
          <Heading>Open questions</Heading>
          <ul className="mt-2 list-disc space-y-1 pl-5">{r.open_questions.map((q, i) => <li key={i}>{q}</li>)}</ul>
        </>
      )}
      {r.chapters.length > 0 && (
        <>
          <Heading>Chapters</Heading>
          <ol className="mt-2 space-y-1">
            {r.chapters.map((c) => (
              <li key={`${c.start_ms}-${c.title}`}>
                <button type="button" onClick={() => seek(c.start_ms)} className="flex gap-4 text-left hover:text-primary">
                  <time className="w-14 shrink-0 font-mono text-xs text-mute">{clock(c.start_ms)}</time>
                  <span>{c.title}</span>
                </button>
              </li>
            ))}
          </ol>
        </>
      )}
      {r.action_items.length > 0 && (
        <>
          <Heading>Action items</Heading>
          <ul className="mt-2 space-y-2">
            {r.action_items.map((a) => (
              <li key={a.id} className="flex flex-wrap items-baseline gap-x-3">
                <span>{a.text}</span>
                {a.due_date && <span className="font-mono text-xs text-mute">due {a.due_date}</span>}
                {a.source_ms !== null && (
                  <button type="button" onClick={() => seek(a.source_ms!)} className="font-mono text-xs text-primary">
                    {clock(a.source_ms)}
                  </button>
                )}
              </li>
            ))}
          </ul>
        </>
      )}
    </>
  )
}
