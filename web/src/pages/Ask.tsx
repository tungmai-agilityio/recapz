import { useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { Link } from 'react-router'
import { ask, clearAskHistory, deleteAskTurn, getAskHistory, type AskFilter, type Citation, NoModel, RateLimited, Unauthorized } from '../api'
import { clock } from '../clock'
import { Button } from '../ui/Button'
import { ConfirmDialog } from '../ui/ConfirmDialog'
import { inputClass } from '../ui/Field'
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

/** `key` is stable while turns come and go; `id` is the saved turn (null until saved, or if saving failed). */
interface Turn { key: number; id: string | null; q: string; a: string; cites: Citation[]; state: 'streaming' | 'done' | 'error'; error: string }

function errorText(err: unknown) {
  if (err instanceof Unauthorized) return 'Sign in to ask.'
  if (err instanceof RateLimited) return 'Too many questions. Try again in a minute.'
  if (err instanceof NoModel) return 'Ask needs a language model. Ask an admin to configure one.'
  return 'Could not get an answer.'
}

export function Ask() {
  const [question, setQuestion] = useState('')
  const [turns, setTurns] = useState<Turn[]>([])
  const [filter, setFilter] = useState<AskFilter>({})
  const filtered = Object.values(filter).some(Boolean)
  const setPart = (k: keyof AskFilter) => (e: { target: { value: string } }) => setFilter((f) => ({ ...f, [k]: e.target.value }))
  const busy = turns.some((t) => t.state === 'streaming')

  const keys = useRef(0)
  const [confirm, setConfirm] = useState<'clear' | Turn | null>(null)
  const [busyDel, setBusyDel] = useState(false)
  const [delError, setDelError] = useState('')

  // Turns are listed newest first, under the question box; a question asked before the history arrives stays on top.
  useEffect(() => {
    getAskHistory().then((h) => setTurns((ts) => [
      ...ts,
      ...h.map((t): Turn => ({ key: ++keys.current, id: t.id, q: t.question, a: t.answer, cites: t.citations, state: 'done', error: '' })).reverse(),
    ])).catch(() => {})
  }, [])

  async function submit(text: string) {
    const q = text.trim()
    if (!q || busy) return
    setQuestion('')
    const key = ++keys.current
    const patch = (f: (t: Turn) => Partial<Turn>) =>
      setTurns((ts) => ts.map((t) => (t.key === key ? { ...t, ...f(t) } : t)))
    setTurns((ts) => [{ key, id: null, q, a: '', cites: [], state: 'streaming', error: '' }, ...ts])
    try {
      const { citations, id } = await ask(q, (tok) => patch((t) => ({ a: t.a + tok })), filter)
      patch(() => ({ cites: citations, id, state: 'done' }))
    } catch (err) {
      patch(() => ({ state: 'error', error: errorText(err) }))
      setQuestion((cur) => cur || q)
    }
  }

  async function confirmed() {
    if (!confirm) return
    setBusyDel(true)
    setDelError('')
    try {
      if (confirm === 'clear') { await clearAskHistory(); setTurns([]) }
      else { await deleteAskTurn(confirm.id!); setTurns((ts) => ts.filter((t) => t.key !== confirm.key)) }
      setConfirm(null)
    } catch {
      setDelError('Could not delete. Try again.')
    } finally {
      setBusyDel(false)
    }
  }

  return (
    <section className="flex max-w-[760px] flex-col gap-5">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Ask</h1>
          <p className="mt-1 text-sm text-mute">Searches every transcript and report you can access. Each answer links to the moment it came from. Your questions are kept for you only; admins cannot read them.</p>
        </div>
      </div>
      <form
        onSubmit={(e) => { e.preventDefault(); submit(question) }}
        className="flex items-center gap-2 rounded-xl border border-hair bg-sheet py-1.5 pl-3.5 pr-1.5"
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
      <div className="flex flex-wrap items-end gap-3 text-sm text-mute">
        <label>From <input type="date" className={`${inputClass} block`} value={filter.from ?? ''} max={filter.to || undefined} onChange={setPart('from')} /></label>
        <label>To <input type="date" className={`${inputClass} block`} value={filter.to ?? ''} min={filter.from || undefined} onChange={setPart('to')} /></label>
        <label>Person <input className={`${inputClass} block`} maxLength={100} placeholder="Who spoke" value={filter.speaker ?? ''} onChange={setPart('speaker')} /></label>
        {filtered && <Button onClick={() => setFilter({})}>Clear filters</Button>}
      </div>
      <div className="flex flex-wrap gap-1.5">
        {SUGGESTIONS.map((s) => (
          <button key={s} type="button" disabled={busy} onClick={() => submit(s)}
            className="rounded-full border border-hair px-3 py-1 text-[0.82rem] text-ink-2 hover:border-ink-2 disabled:opacity-50">{s}</button>
        ))}
      </div>
      {turns.length > 0 && (
        <div className="flex items-center justify-between gap-3">
          <h2 className="text-sm font-semibold uppercase tracking-wider text-mute">History</h2>
          {turns.some((t) => t.id) && <Button disabled={busy} onClick={() => { setDelError(''); setConfirm('clear') }}>Clear history</Button>}
        </div>
      )}
      {turns.length > 0 && <div className="frame flex flex-col gap-5">{turns.map((t, ti) => (
        <div key={t.key} className="flex flex-col gap-3.5">
          {ti > 0 && <div className="h-px bg-hair" />}
          <div className="flex items-start justify-between gap-3">
            <p className="text-xl font-semibold leading-snug tracking-tight">{t.q}</p>
            {t.id && <Button variant="ghost" aria-label="Delete this question" title="Delete this question" onClick={() => { setDelError(''); setConfirm(t) }}>Delete</Button>}
          </div>
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
                  to={c.meeting_id ? `/meetings/${c.meeting_id}?t=${c.start_ms ?? 0}` : c.document_id ? `/documents/${c.document_id}` : null}
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
      {confirm && (
        <ConfirmDialog title={confirm === 'clear' ? 'Clear Ask history?' : 'Delete this question?'} confirmLabel={confirm === 'clear' ? 'Clear history' : 'Delete'}
          busy={busyDel} error={delError || undefined} onConfirm={confirmed} onCancel={() => setConfirm(null)}>
          {confirm === 'clear' ? 'All your saved questions and answers are deleted for good.' : 'This question and its answer are deleted for good.'}
        </ConfirmDialog>
      )}
    </section>
  )
}
