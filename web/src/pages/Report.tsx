import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { Link, useNavigate, useParams, useSearchParams } from 'react-router'
import { clock } from '../clock'
import { meetingTime } from './meeting-days'
import { ConfirmDialog } from '../ui/ConfirmDialog'
import { Face } from '../ui/Face'
import { Ribbon, SpeakerBar } from '../ui/Ribbon'
import { colour, speakerIndex } from '../ui/speaker-colour'
import { Tabs } from '../ui/Tabs'
import { Menu, MenuItem } from '../ui/Menu'
import { useToast } from '../ui/toast-context'
import { Button } from '../ui/Button'
import { inputClass } from '../ui/Field'
import { deleteMeeting, getMeeting, getReport, getTranscript, renameMeeting, Unauthorized, type MeetingDetail, type ReportData } from '../api'

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
  const [confirming, setConfirming] = useState(false)
  const toast = useToast()
  const nav = useNavigate()
  const qc = useQueryClient()
  const del = useMutation({
    mutationFn: () => deleteMeeting(id),
    onSuccess: () => {
      toast('Meeting deleted')
      void qc.invalidateQueries({ queryKey: ['meetings'] })
      nav('/')
    },
  })
  const [draft, setDraft] = useState<string | null>(null) // non-null while the title is being edited
  const rename = useMutation({
    mutationFn: (title: string) => renameMeeting(id, title),
    onSuccess: ({ title }) => {
      qc.setQueryData(['meeting', id], (m: MeetingDetail | undefined) => m && { ...m, title })
      void qc.invalidateQueries({ queryKey: ['meetings'] })
      setDraft(null)
    },
  })
  const audio = useRef<HTMLAudioElement>(null)
  const [now, setNow] = useState(params.has('t') && Number.isFinite(at) ? at : 0) // playhead, ms
  const [playing, setPlaying] = useState(false)
  const [noAudio, setNoAudio] = useState(false)
  const [length, setLength] = useState(0) // ms, from the audio file
  const [tab, setTab] = useState(params.has('t') ? 'transcript' : 'summary')
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
  const seek = (ms: number, play = true) => {
    if (!audio.current) return
    audio.current.currentTime = ms / 1000
    setNow(ms)
    if (play) void audio.current.play()?.catch(() => {})
  }
  const toggle = () => {
    const a = audio.current
    if (!a || noAudio) return
    if (a.paused) void a.play()?.catch(() => {})
    else a.pause()
  }
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== ' ' || (e.target as HTMLElement).closest('input,textarea,select,button,a,[role=tab],[role=slider]')) return
      e.preventDefault()
      toggle()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  })
  // Open at the cited moment once the transcript is on screen.
  useEffect(() => {
    if (!transcript.data || !params.has('t') || !Number.isFinite(at)) return
    if (audio.current) audio.current.currentTime = at / 1000
    document.querySelector('[aria-current]')?.scrollIntoView?.({ block: 'center' })
  }, [transcript.data, at, params])

  const segs = transcript.data ?? []
  const ids = [...new Set(segs.map((s) => s.speaker_id).filter((x): x is string => x !== null))]
  const speakerOf = (id: string | null) => speakerIndex(meeting.data?.speakers, id)
  const ribbon = segs.map((s) => ({ speaker: speakerOf(s.speaker_id), weight: Math.max(s.end_ms - s.start_ms, 1) }))
  const names = new Map(segs.filter((s) => s.speaker_id !== null).map((s) => [s.speaker_id, s.speaker_name ?? 'Unknown']))
  const spoken = ids.map((sid) => ({
    id: sid, index: speakerOf(sid), name: names.get(sid)!,
    ms: segs.filter((s) => s.speaker_id === sid).reduce((n, s) => n + Math.max(s.end_ms - s.start_ms, 0), 0),
  })).sort((a, b) => b.ms - a.ms)
  const talk = { people: spoken, all: segs.reduce((n, s) => n + Math.max(s.end_ms - s.start_ms, 0), 0) }
  const total = length || (segs.length ? Math.max(...segs.map((s) => s.end_ms)) : 0)
  if (meeting.error instanceof Unauthorized) return <Link to="/" className="text-primary">Sign in</Link>
  if (meeting.isError) return <p className="text-mute">Meeting not found.</p>
  if (!meeting.data) return <p className="text-mute">Loading…</p>
  const m = meeting.data

  return (
    <article>
      <div className="flex items-center justify-between">
        <Link to="/" className="text-sm text-mute hover:text-primary">← All meetings</Link>
        <Menu label="Meeting actions" side="below" trigger="⋯">
          <MenuItem onClick={() => { rename.reset(); setDraft(m.title) }}>Rename</MenuItem>
          <MenuItem className="text-bad" onClick={() => { del.reset(); setConfirming(true) }}>Delete</MenuItem>
        </Menu>
      </div>
      {draft === null ? (
        <h1 className="mt-2 text-2xl font-semibold tracking-tight">{m.title}</h1>
      ) : (
        <form className="mt-2" onSubmit={(e) => { e.preventDefault(); if (draft.trim() && !rename.isPending) rename.mutate(draft.trim()) }}>
          <div className="flex gap-2">
            <input autoFocus aria-label="Meeting title" maxLength={200} value={draft} className={`${inputClass} min-w-0 flex-1 text-lg font-semibold`}
              aria-invalid={rename.isError || undefined}
              onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape') setDraft(null) }}
              onFocus={(e) => e.currentTarget.select()} />
            <Button type="submit" variant="primary" disabled={!draft.trim() || rename.isPending}>Save</Button>
            <Button onClick={() => setDraft(null)}>Cancel</Button>
          </div>
          {rename.error && <p role="alert" className="mt-1 text-xs text-bad">{rename.error.message.replace(/^\d+ /, '')}</p>}
        </form>
      )}
      <p className="mt-2 text-sm text-mute">
        {new Date(meetingTime(m)).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })}
        {total > 0 && <> · {clock(total)}</>} · {m.platform}
        {spoken.length > 0 && (
          <span className="ml-2 inline-flex translate-y-1.5">
            {[...spoken].sort((a, b) => (a.index ?? 99) - (b.index ?? 99)).map((p, i) => <span key={p.id} className={i ? '-ml-1.5' : ''}><Face name={p.name} speaker={p.index} /></span>)}
          </span>
        )}
      </p>
      {m.status === 'failed' && <p role="alert" className="mt-3 text-sm text-bad">{m.error ?? 'Processing failed.'}</p>}

      {confirming && (
        <ConfirmDialog title="Delete this meeting?" confirmLabel="Delete meeting" busy={del.isPending}
          error={del.error ? del.error.message.replace(/^\d+ /, '') : undefined}
          onConfirm={() => del.mutate()} onCancel={() => setConfirming(false)}>
          The audio, transcript, report and Ask search entries are removed permanently. This cannot be undone.
        </ConfirmDialog>
      )}

      <audio
        ref={audio}
        preload="metadata"
        src={`/api/meetings/${id}/audio`}
        onTimeUpdate={(e) => setNow(e.currentTarget.currentTime * 1000)}
        onLoadedMetadata={(e) => { setNoAudio(false); if (Number.isFinite(e.currentTarget.duration)) setLength(e.currentTarget.duration * 1000) }}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onError={() => setNoAudio(true)}
      />
      {noAudio ? (
        <p className="mt-5 text-sm text-mute">No audio is available for this meeting yet.</p>
      ) : (
        <div className="mt-5 flex items-center gap-3.5">
          <button type="button" aria-label={playing ? 'Pause' : 'Play'} onClick={toggle}
            className="flex size-9 flex-none items-center justify-center rounded-full bg-primary text-primary-fg">
            <svg viewBox="0 0 24 24" className="size-3.5 fill-current" aria-hidden>
              <path d={playing ? 'M6 5h4v14H6zM14 5h4v14h-4z' : 'M7 4l13 8-13 8z'} />
            </svg>
          </button>
          <div className="min-w-0 flex-1">
            {total > 0 && (
              <Ribbon segments={ribbon} variant="scrubber" total={total} now={now} onSeek={(ms) => seek(ms, false)}
                ticks={report.data?.chapters.map((c) => ({ ms: c.start_ms, title: c.title }))} />
            )}
          </div>
          <span className="font-mono text-sm tabular-nums">{clock(now)} <span className="text-mute">/ {clock(total)}</span></span>
        </div>
      )}

      <div className="mt-6">
        <Tabs label="Report sections" value={tab} onChange={setTab} tabs={[{ value: 'summary', label: 'Summary' }, { value: 'transcript', label: 'Transcript' }]} />
      </div>
      {tab === 'summary' ? (
        <div role="tabpanel" className="mt-6">
          {report.data ? (
            <ReportSections r={report.data} talk={talk} language={m.language} seek={seek} open={(ms) => { setTab('transcript'); seek(ms) }} />
          ) : (
            <p className="text-mute">
              {m.status === 'ready' ? 'No report was written for this meeting.'
                : m.status === 'failed' ? 'No report, because processing failed.'
                : 'The report will appear when processing finishes.'}
            </p>
          )}
        </div>
      ) : (
        <div role="tabpanel" className="frame mt-4">
          {transcript.isLoading && <p className="text-mute">Loading…</p>}
          {transcript.isError && <p className="text-bad">Could not load the transcript.</p>}
          {transcript.data?.length === 0 && (
            <p className="text-mute">
              {m.status === 'ready' ? 'No speech was found.' : 'The transcript will appear when processing finishes.'}
            </p>
          )}
          <ol>
            {segs.map((s) => {
              const active = now >= s.start_ms && now < s.end_ms
              return (
                <li key={s.id}>
                  <button type="button" aria-current={active || undefined} onClick={() => seek(s.start_ms)}
                    className={`grid w-full grid-cols-[3px_3.5rem_minmax(0,1fr)] gap-3 rounded-md py-2.5 pr-2 text-left hover:bg-inset ${active ? 'bg-signal-wash' : ''}`}>
                    <SpeakerBar speaker={speakerOf(s.speaker_id)} />
                    <time className="pt-0.5 font-mono text-xs text-mute">{clock(s.start_ms)}</time>
                    <span>
                      {s.speaker_id !== null && <span className="mr-2 text-[0.82rem] font-semibold">{s.speaker_name ?? 'Unknown'}</span>}
                      {s.text}
                    </span>
                  </button>
                </li>
              )
            })}
          </ol>
        </div>
      )}
    </article>
  )
}

const Heading = ({ children }: { children: string }) => (
  <h2 className="text-lg font-semibold">{children}</h2>
)

function ReportSections({ r, talk, language, seek, open }: { r: ReportData; talk: { people: { id: string; index: number | null; name: string; ms: number }[]; all: number }; language: string; seek: (ms: number) => void; open: (ms: number) => void }) {
  const done = r.action_items.filter((a) => a.done).length
  const time = (ms: number) => (
    <button type="button" onClick={() => seek(ms)} className="font-mono text-xs text-mute hover:text-primary">{clock(ms)}</button>
  )
  return (
    <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_16rem]">
      <div className="frame space-y-6">
        <p className="max-w-[66ch] leading-7">{r.summary}</p>
        {r.decisions.length > 0 && (
          <section className="space-y-2">
            <Heading>Decisions</Heading>
            <ul className="list-disc space-y-1 pl-5">{r.decisions.map((d, i) => <li key={i}>{d}</li>)}</ul>
          </section>
        )}
        {r.open_questions.length > 0 && (
          <section className="space-y-2">
            <Heading>Open questions</Heading>
            <ul className="list-disc space-y-1 pl-5">{r.open_questions.map((q, i) => <li key={i}>{q}</li>)}</ul>
          </section>
        )}
        {r.action_items.length > 0 && (
          <section className="space-y-2">
            <Heading>{`Action items · ${done}/${r.action_items.length} done`}</Heading>
            <ul>
              {r.action_items.map((a) => (
                <li key={a.id} className="flex items-start gap-2.5 border-t border-hair-2 py-2.5 first:border-t-0">
                  <span aria-hidden className={`mt-1 size-3.5 flex-none rounded-sm border ${a.done ? 'border-ink bg-ink' : 'border-mute'}`} />
                  <span className={a.done ? 'text-mute line-through' : ''}>
                    {a.done && <span className="sr-only">Done: </span>}{a.text}
                    {a.due_date && <span className="block font-mono text-xs text-mute">due {a.due_date}</span>}
                  </span>
                  {a.source_ms !== null && <span className="ml-auto">{time(a.source_ms)}</span>}
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
      <aside className="frame space-y-6">
        {r.chapters.length > 0 && (
          <section className="space-y-1">
            <Heading>Chapters</Heading>
            <ol>
              {r.chapters.map((c) => (
                <li key={`${c.start_ms}-${c.title}`}>
                  <button type="button" onClick={() => open(c.start_ms)} className="grid w-full grid-cols-[3rem_1fr] gap-2 py-1.5 text-left text-sm hover:underline">
                    <span className="font-mono text-mute">{clock(c.start_ms)}</span>
                    <span>{c.title}</span>
                  </button>
                </li>
              ))}
            </ol>
          </section>
        )}
        {talk.people.length > 0 && talk.all > 0 && (
          <section className="space-y-1">
            <Heading>Talk time</Heading>
            <ul>
              {talk.people.map((p) => (
                <li key={p.id} className="grid grid-cols-[70px_1fr_38px] items-center gap-2.5 py-0.5 text-[0.84rem]">
                  <span className="truncate">{p.name}</span>
                  <span data-testid="talk-bar" style={{ width: `${(p.ms / talk.people[0].ms) * 100}%` }} className={`h-1.5 rounded-[2px] ${colour(p.index)}`} />
                  <span className="font-mono text-mute">{Math.round((p.ms / talk.all) * 100)}%</span>
                </li>
              ))}
            </ul>
          </section>
        )}
        <section className="space-y-1">
          <Heading>Details</Heading>
          <p className="text-sm text-mute">Language: {language}</p>
        </section>
      </aside>
    </div>
  )
}
