import { useState } from 'react'
import { Link } from 'react-router'
import { ask, type Citation, Unauthorized } from '../api'
import { clock } from '../clock'

export function Ask() {
  const [question, setQuestion] = useState('')
  const [answer, setAnswer] = useState('')
  const [citations, setCitations] = useState<Citation[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    const q = question.trim()
    if (!q || busy) return
    setBusy(true)
    setError('')
    setAnswer('')
    setCitations([])
    try {
      setCitations(await ask(q, (t) => setAnswer((a) => a + t)))
    } catch (err) {
      setError(err instanceof Unauthorized ? 'Sign in to ask.' : 'Could not get an answer.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <section>
      <h1 className="text-2xl font-semibold tracking-tight">Ask</h1>
      <p className="text-sm text-mute">Questions are answered from the meetings you can see.</p>
      <form onSubmit={submit} className="mt-4 flex gap-2">
        <input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          maxLength={1000}
          aria-label="Question"
          placeholder="What did we decide about pricing?"
          className="flex-1 rounded-md border border-hair px-3 py-2"
        />
        <button type="submit" disabled={busy} className="rounded-md bg-primary px-4 py-2 text-white disabled:opacity-50">
          {busy ? 'Asking…' : 'Ask'}
        </button>
      </form>
      {error && <p role="alert" className="mt-4 text-sm text-red-700">{error}</p>}
      {answer && <p className="mt-6 whitespace-pre-wrap" aria-live="polite">{answer}</p>}
      <ul className="mt-4 space-y-2">
        {citations.map((c, i) => (
          <li key={i}>
            <Link
              to={`/meetings/${c.meeting_id}?t=${c.start_ms}`}
              className="block rounded-md border border-hair p-3 hover:bg-sheet"
            >
              <span className="text-sm font-medium">{c.title}</span>
              <time className="ml-2 font-mono text-xs text-mute">{clock(c.start_ms)}</time>
              <span className="mt-1 block text-sm text-mute">{c.quote}</span>
            </Link>
          </li>
        ))}
      </ul>
    </section>
  )
}
