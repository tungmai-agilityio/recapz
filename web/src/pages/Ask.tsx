import { useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { Link } from 'react-router'
import { ask, type Citation, NoModel, RateLimited, Unauthorized } from '../api'
import { clock } from '../clock'
import { Button } from '../ui/Button'
import { Ribbon } from '../ui/Ribbon'

const SUGGESTIONS = ['What decisions were made recently?', 'What are the open action items?', 'What was said about deadlines?']

/** A citation card: a link into the meeting, or a plain card for a document. */
function Cite({ to, ...p }: { to: string | null; id: string; className: string; children: ReactNode }) {
  return to ? <Link to={to} {...p} /> : <div {...p} />
}

/** The answer with each `[n]` as a superscript. While streaming the cards are not known yet, so markers are plain
 *  numbers (an unfinished `[1` waits); once done a marker links to the card that cites it and is dropped without one. */
function AnswerText({ text, cites, ti, streaming }: { text: string; cites: Citation[]; ti: number; streaming: boolean }) {
  const t = streaming ? text.replace(/\[\d*$/, '') : text
  const out: ReactNode[] = []
  let last = 0
  for (const m of t.matchAll(/\[(\d+)\]/g)) {
    out.push(t.slice(last, m.index))
    last = m.index + m[0].length
    const n = Number(m[1])
    const card = cites.slice(0, 3).findIndex((c) => c.n === n)
    if (streaming) out.push(<sup key={m.index} className="ml-0.5 text-[0.68rem]">{n}</sup>)
    else if (card >= 0) out.push(
      <sup key={m.index} className="ml-0.5 text-[0.68rem]">
        <a href={`#cite-${ti}-${card}`} className="rounded-sm bg-hair-2 px-1 font-mono no-underline">{n}</a>
      </sup>)
  }
  return <>{out}{t.slice(last)}</>
}

/** Where the cited moment sits in the meeting, as a pin on the speaker ribbon. */
function CardRibbon({ c }: { c: Citation }) {
  if (!c.ribbon?.length || !c.duration_ms || c.start_ms == null) return null
  const at = Math.min(100, Math.max(0, (c.start_ms / c.duration_ms) * 100))
  return (
    <span className="relative block py-1" role="img" aria-label={`Speaker ribbon, moment at ${clock(c.start_ms)}`}>
      <Ribbon segments={c.ribbon} variant="card" />
      <span data-testid="pin" style={{ left: `${at}%` }} className="absolute inset-y-0 w-0.5 bg-signal" />
    </span>
  )
}

interface Turn { q: string; a: string; cites: Citation[]; state: 'streaming' | 'done' | 'error'; error: string }

function errorText(err: unknown) {
  if (err instanceof Unauthorized) return 'Sign in to ask.'
  if (err instanceof RateLimited) return 'Too many questions. Try again in a minute.'
  if (err instanceof NoModel) return 'Ask needs a language model. Ask an admin to configure one.'
  return 'Could not get an answer.'
}

export function Ask() {
  const [question, setQuestion] = useState('')
  const [turns, setTurns] = useState<Turn[]>([])
  const busy = turns.some((t) => t.state === 'streaming')
  const bottom = useRef<HTMLDivElement>(null)
  useEffect(() => { bottom.current?.scrollIntoView?.({ block: 'end' }) }, [turns.length])

  async function submit(text: string) {
    const q = text.trim()
    if (!q || busy) return
    setQuestion('')
    const i = turns.length
    const patch = (f: (t: Turn) => Partial<Turn>) =>
      setTurns((ts) => ts.map((t, j) => (j === i ? { ...t, ...f(t) } : t)))
    setTurns((ts) => [...ts, { q, a: '', cites: [], state: 'streaming', error: '' }])
    try {
      const cites = await ask(q, (tok) => patch((t) => ({ a: t.a + tok })))
      patch(() => ({ cites, state: 'done' }))
    } catch (err) {
      patch(() => ({ state: 'error', error: errorText(err) }))
      setQuestion((cur) => cur || q)
    }
  }

  return (
    <section className="flex max-w-[760px] flex-col gap-5">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Ask</h1>
        <p className="mt-1 text-sm text-mute">Searches every transcript and report you can access. Each answer links to the moment it came from.</p>
      </div>
      {turns.length > 0 && <div className="frame flex flex-col gap-5">{turns.map((t, ti) => (
        <div key={ti} className="flex flex-col gap-3.5">
          {ti > 0 && <div className="h-px bg-hair" />}
          <p className="text-xl font-semibold leading-snug tracking-tight">{t.q}</p>
          {(t.state !== 'error' || t.a) && (
            <p className="whitespace-pre-wrap leading-7" aria-live="polite">
              {t.a ? <AnswerText text={t.a} cites={t.cites} ti={ti} streaming={t.state === 'streaming'} /> : <span className="text-mute">Searching your meetings…</span>}
            </p>
          )}
          {t.state === 'error' && <p role="alert" className="text-sm text-bad">{t.error}</p>}
          {t.state === 'done' && t.cites.length === 0 && <p className="text-sm text-mute">No matching moments found.</p>}
          {t.cites.length > 0 && (
            <div className="grid gap-2.5 @xl:grid-cols-3">
              {t.cites.slice(0, 3).map((c, i) => (
                <Cite
                  key={i}
                  id={`cite-${ti}-${i}`}
                  to={c.meeting_id ? `/meetings/${c.meeting_id}?t=${c.start_ms ?? 0}` : null}
                  className="flex min-w-0 flex-col gap-2 rounded-lg border border-hair bg-inset p-3 hover:border-ink-2"
                >
                  <span className="flex justify-between font-mono text-xs"><span>{c.n ?? ''}</span><time className="text-mute">{c.start_ms === null ? (c.answered ? 'Document' : 'Document, not sent to the model') : clock(c.start_ms)}</time></span>
                  <b className="text-sm font-semibold">{c.title}</b>
                  <CardRibbon c={c} />
                  {c.speaker && <span className="text-xs text-mute">{c.speaker}</span>}
                  <span className="line-clamp-2 text-[0.82rem] text-ink-2">{c.quote}</span>
                </Cite>
              ))}
            </div>
          )}
        </div>
      ))}</div>}
      <div className="flex flex-wrap gap-1.5">
        {SUGGESTIONS.map((s) => (
          <button key={s} type="button" disabled={busy} onClick={() => submit(s)}
            className="rounded-full border border-hair px-3 py-1 text-[0.82rem] text-ink-2 hover:border-ink-2 disabled:opacity-50">{s}</button>
        ))}
      </div>
      <form
        onSubmit={(e) => { e.preventDefault(); submit(question) }}
        className="sticky bottom-4 flex items-center gap-2 rounded-xl border border-hair bg-sheet py-1.5 pl-3.5 pr-1.5 shadow-lg"
      >
        <input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          maxLength={1000}
          aria-label="Question"
          placeholder="Ask anything"
          autoComplete="off"
          className="min-w-0 flex-1 bg-transparent py-2 outline-none"
        />
        <Button type="submit" variant="primary" disabled={busy}>Ask</Button>
      </form>
      <div ref={bottom} />
    </section>
  )
}
