import { useQuery } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import type { KeyboardEvent as ReactKeyboardEvent } from 'react'
import { Link, useParams } from 'react-router'
import { getMeeting, liveAsk, NoModel, NotLive, RateLimited, Unauthorized, watchLive, type Citation, type LiveEvent } from '../api'
import { clock } from '../clock'
import { Badge } from '../ui/Badge'
import { Button } from '../ui/Button'
import { inputClass } from '../ui/Field'

interface Line { at_ms: number; text: string }
interface Card { id: number; context: string; ms: number; answer: string; cites: Citation[]; error: string }

function errorText(err: unknown) {
  if (err instanceof Unauthorized) return 'Sign in to ask.'
  if (err instanceof RateLimited) return 'Too many questions. Try again in a minute.'
  if (err instanceof NoModel) return 'Ask needs a language model. Ask an admin to configure one.'
  if (err instanceof NotLive) return 'This call is no longer live.'
  return 'Could not get an answer.'
}

/** Answers, sources and the question box. Only the member who asks sees these. */
function AskPanel({ id, picked, onClear, ended }: { id: string; picked: Line | null; onClear: () => void; ended: boolean }) {
  const [extra, setExtra] = useState('')
  const [cards, setCards] = useState<Card[]>([])
  const [waiting, setWaiting] = useState(false)
  const next = useRef(0)

  async function askNow() {
    if (waiting || ended) return
    const started = Date.now()
    const context = picked ? `Line at ${clock(picked.at_ms)}` : 'Last 60 s'
    const body = { ...(picked ? { at_ms: picked.at_ms } : {}), ...(extra.trim() ? { text: extra.trim() } : {}) }
    setWaiting(true)
    const card = { id: next.current++, context, ms: 0, answer: '', cites: [] as Citation[], error: '' }
    try {
      const r = await liveAsk(id, body)
      setCards((cs) => [{ ...card, answer: r.answer, cites: r.citations, ms: Date.now() - started }, ...cs])
      setExtra('')
    } catch (err) {
      setCards((cs) => [{ ...card, error: errorText(err), ms: Date.now() - started }, ...cs])
    } finally {
      setWaiting(false)
    }
  }

  // The A key asks, unless the member is typing somewhere.
  const ask = useRef(askNow)
  useEffect(() => { ask.current = askNow })
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null
      if (e.key.toLowerCase() !== 'a' || e.metaKey || e.ctrlKey || e.altKey) return
      if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return
      e.preventDefault()
      void ask.current()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [])

  return (
    <aside aria-label="Live Ask" className="frame flex flex-col gap-3.5 self-start">
      <div>
        <h2 className="text-base font-semibold tracking-tight">Live Ask</h2>
        <p className="text-xs text-mute">Only you see these.</p>
      </div>
      <p className="flex flex-wrap items-center gap-2 text-sm text-ink-2">
        <span>{picked ? `Asking about the line at ${clock(picked.at_ms)}` : 'Asking about the last 60 s'}</span>
        {picked && <button type="button" onClick={onClear} className="text-xs underline">use last 60 s</button>}
      </p>
      <input value={extra} onChange={(e) => setExtra(e.target.value)} maxLength={1000} aria-label="Extra words"
        placeholder="Add words to the question (optional)" autoComplete="off" className={inputClass}
        onKeyDown={(e: ReactKeyboardEvent) => { if (e.key === 'Enter') void askNow() }} />
      <Button variant="primary" onClick={() => void askNow()} disabled={waiting || ended} aria-keyshortcuts="A" className="self-start">
        {waiting ? 'Asking…' : 'Ask now'}<kbd className="rounded border border-primary-fg/40 px-1 font-mono text-[0.68rem]">A</kbd>
      </Button>
      {waiting && <p role="status" className="text-sm text-mute">Searching your documents and meetings…</p>}
      {cards.map((c) => {
        const answered = c.cites.filter((x) => x.answered)
        const passages = c.cites.filter((x) => !x.answered)
        return (
          <article key={c.id} className="flex flex-col gap-2 border-t border-hair pt-3">
            <p className="flex justify-between text-xs text-mute"><span>{c.context}</span><span>{(c.ms / 1000).toFixed(1)} s</span></p>
            {c.error ? <p role="alert" className="text-sm text-bad">{c.error}</p> : <p className="leading-6">{c.answer}</p>}
            {answered.map((x, i) => <Source key={i} c={x} />)}
            {passages.length > 0 && (
              <div className="flex flex-col gap-1.5">
                <Badge>Passages only</Badge>
                <p className="text-xs text-mute">These documents are not sent to the language model, so there is no generated answer from them.</p>
                {passages.map((x, i) => <Source key={i} c={x} />)}
              </div>
            )}
          </article>
        )
      })}
    </aside>
  )
}

function Source({ c }: { c: Citation }) {
  const body = (
    <>
      <span className="flex justify-between text-xs"><b className="font-semibold">{c.title}</b><span className="text-mute">{c.start_ms === null ? 'Document' : clock(c.start_ms)}</span></span>
      <span className="line-clamp-2 text-[0.82rem] text-ink-2">{c.quote}</span>
    </>
  )
  const cls = 'flex min-w-0 flex-col gap-1 rounded-lg border border-hair bg-inset p-2.5'
  return c.meeting_id ? <Link to={`/meetings/${c.meeting_id}?t=${c.start_ms ?? 0}`} className={`${cls} hover:border-ink-2`}>{body}</Link> : <div className={cls}>{body}</div>
}

/** The live transcript of a call in progress, with Ask now beside it. */
export function Live() {
  const { id = '' } = useParams()
  const meeting = useQuery({ queryKey: ['meeting', id], queryFn: () => getMeeting(id) })
  const [lines, setLines] = useState<Line[]>([])
  const [partial, setPartial] = useState('')
  const [state, setState] = useState<'live' | 'unavailable' | 'ended' | 'not-live'>('live')
  const [picked, setPicked] = useState<Line | null>(null)
  const bottom = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const stop = new AbortController()
    const on = (e: LiveEvent) => {
      if (e.type === 'final') { setLines((ls) => [...ls, { at_ms: e.at_ms, text: e.text }]); setPartial('') }
      else if (e.type === 'partial') setPartial(e.text)
      else if (e.type === 'unavailable') setState('unavailable')
      else if (e.type === 'end') setState('ended')
    }
    watchLive(id, on, stop.signal)
      // The stream closing without an `end` event means the connection dropped, not that the call is over.
      .then(() => setState((s) => (s === 'ended' ? s : 'unavailable')))
      .catch((err) => { if (!stop.signal.aborted) setState(err instanceof NotLive ? 'not-live' : 'unavailable') })
    return () => stop.abort()
  }, [id])
  useEffect(() => { bottom.current?.scrollIntoView?.({ block: 'end' }) }, [lines.length])

  const title = meeting.data?.title ?? 'Live meeting'
  if (state === 'not-live')
    return (
      <section className="flex max-w-[760px] flex-col gap-3">
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
        <p className="text-sm text-mute">This meeting is not being recorded right now.</p>
        <Link to={`/meetings/${id}`} className="text-sm underline">Open the meeting</Link>
      </section>
    )
  return (
    <section className="grid items-start gap-5 min-[1000px]:grid-cols-[minmax(0,1fr)_340px]">
      <div className="flex min-w-0 flex-col gap-4">
        <div className="flex flex-wrap items-center gap-2.5">
          <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
          {state === 'ended' ? <Badge>Call ended</Badge> : <Badge tone="signal">Recording</Badge>}
        </div>
        {state === 'unavailable' && <p role="status" className="text-sm text-mute">Live transcript unavailable. Recording goes on and the full transcript follows after the call.</p>}
        {state === 'ended' && <p role="status" className="text-sm text-mute">The call has ended. <Link to={`/meetings/${id}`} className="underline">Open the meeting</Link></p>}
        <div className="frame flex max-h-[60vh] min-h-40 flex-col gap-1.5 overflow-y-auto" aria-label="Live transcript">
          {lines.length === 0 && !partial && <p className="text-sm text-mute">Waiting for the first words…</p>}
          {lines.map((l, i) => (
            <button key={i} type="button" onClick={() => setPicked(l)} aria-pressed={picked?.at_ms === l.at_ms}
              className={`flex gap-3 rounded-md px-2 py-1 text-left hover:bg-hair-2 ${picked?.at_ms === l.at_ms ? 'bg-signal-wash' : ''}`}>
              <time className="w-12 shrink-0 font-mono text-xs text-mute">{clock(l.at_ms)}</time><span>{l.text}</span>
            </button>
          ))}
          {partial && <p className="px-2 py-1 text-mute" aria-live="polite">{partial}</p>}
          <div ref={bottom} />
        </div>
      </div>
      <AskPanel id={id} picked={picked} onClear={() => setPicked(null)} ended={state === 'ended'} />
    </section>
  )
}
