import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router'
import { useEffect, useRef, useState } from 'react'
import { ConfirmDialog } from '../ui/ConfirmDialog'
import { Badge } from '../ui/Badge'
import { Button } from '../ui/Button'
import { Columns } from '../ui/Columns'
import { PageHead } from '../ui/PageHead'
import { useToast } from '../ui/toast-context'
import { LiveCard } from './LiveCard'
import { NextUp } from './NextUp'
import { RecordDialog } from './Record'
import { useSpeechProvider } from '../speech'
import { Face } from '../ui/Face'
import { Ribbon } from '../ui/Ribbon'
import { groupByDay, hoursLabel, meetingTime, minutesLabel, statusBadge } from './meeting-days'
import { Field, inputClass } from '../ui/Field'
import { deleteMeeting, devLogin, getMeetings, joinMeeting, Unauthorized, type Meeting } from '../api'

const at = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
const thisMonth = (ms: number) => { const a = new Date(ms), b = new Date(); return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() }

const counts = (all: Meeting[], provider: string) => {
  const month = all.filter((m) => thisMonth(meetingTime(m)))
  const ms = month.reduce((n, m) => n + (m.duration_ms ?? 0), 0)
  return [`${month.length} meetings this month`, ms > 0 && `${hoursLabel(ms)} recorded`, `audio is processed by ${provider}`].filter(Boolean).join(' · ')
}

/** Shown only by the Vite dev server (`import.meta.env.DEV`), never in a production build. */
function DevLogin({ onDone }: { onDone: () => void }) {
  const [email, setEmail] = useState('')
  const [failed, setFailed] = useState(false)
  return (
    <form
      className="mt-6 border-t border-hair pt-4"
      onSubmit={(e) => {
        e.preventDefault()
        devLogin(email.trim()).then(onDone, () => setFailed(true))
      }}
    >
      <label className="text-sm text-mute" htmlFor="dev-email">Dev login (local only)</label>
      <div className="mt-1 flex gap-2">
        <input id="dev-email" type="email" required value={email} onChange={(e) => setEmail(e.target.value)}
          placeholder="bootstrap_admin_email" className="flex-1 rounded-lg border border-hair px-3 py-2 text-sm" />
        <button className="rounded-lg border border-hair px-4 py-2 text-sm font-medium">Sign in</button>
      </div>
      {failed && <p role="alert" className="mt-2 text-sm text-bad">Rejected. Set dev_login = true and use the bootstrap admin email.</p>}
    </form>
  )
}

function SendNotetaker({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient()
  const toast = useToast()
  const [url, setUrl] = useState('')
  const [title, setTitle] = useState('')
  const form = useRef<HTMLFormElement>(null)
  const close = useRef(onClose)
  useEffect(() => { close.current = onClose })
  // Escape from anywhere closes; Tab wraps inside the dialog.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') return close.current()
      if (e.key !== 'Tab' || !form.current) return
      const f = [...form.current.querySelectorAll<HTMLElement>('input, button:not([disabled])')]
      const [first, last] = [f[0], f[f.length - 1]]
      if (!form.current.contains(document.activeElement)) { e.preventDefault(); first.focus() }
      else if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus() }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus() }
    }
    document.addEventListener('keydown', key)
    return () => document.removeEventListener('keydown', key)
  }, [])
  const send = useMutation({
    mutationFn: () => joinMeeting(url.trim(), title.trim()),
    onSuccess: () => {
      toast('Notetaker is on its way')
      onClose()
      return qc.invalidateQueries({ queryKey: ['meetings'] })
    },
  })
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-ink/40 p-4">
    <form ref={form} role="dialog" aria-modal="true" className="flex w-full max-w-md flex-col gap-3 rounded-lg border border-hair bg-sheet p-5 shadow-xl" aria-label="Send notetaker"
      onSubmit={(e) => { e.preventDefault(); send.mutate() }}>
      <Field label="Meeting link" hint="A Google Meet or Zoom link."
        error={send.error ? send.error.message.replace(/^\d+ /, '') : undefined}>
        {(p) => <input {...p} type="url" required autoFocus value={url} onChange={(e) => setUrl(e.target.value)}
          placeholder="https://meet.google.com/abc-defg-hij" className={inputClass} />}
      </Field>
      <Field label="Title (optional)">
        {(p) => <input {...p} maxLength={200} value={title} onChange={(e) => setTitle(e.target.value)} className={inputClass} />}
      </Field>
      <div className="flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" type="submit" disabled={send.isPending}>{send.isPending ? 'Sending…' : 'Send notetaker'}</Button>
      </div>
    </form>
    </div>
  )
}

export function Meetings() {
  const qc = useQueryClient()
  const { data, error, isLoading } = useQuery({
    queryKey: ['meetings'],
    queryFn: getMeetings,
    retry: (n, e) => !(e instanceof Unauthorized) && n < 2,
  })
  const provider = useSpeechProvider()
  const toast = useToast()
  const [sending, setSending] = useState(false)
  const [recording, setRecording] = useState(false)
  const [deleting, setDeleting] = useState<{ id: string; title: string } | null>(null)
  const del = useMutation({
    mutationFn: deleteMeeting,
    onSuccess: () => {
      toast('Meeting deleted')
      setDeleting(null)
      return qc.invalidateQueries({ queryKey: ['meetings'] })
    },
  })

  if (error instanceof Unauthorized)
    return (
      <div className="rounded-xl border border-hair bg-sheet p-8">
        <h1 className="text-xl font-semibold">Sign in to Recapz</h1>
        <p className="mt-2 text-mute">Access is by invitation. Use the Google account you were invited with.</p>
        <a href="/auth/google" className="mt-4 inline-block rounded-lg bg-primary px-4 py-2 font-medium text-primary-fg">
          Sign in with Google
        </a>
        {import.meta.env.DEV && <DevLogin onDone={() => qc.invalidateQueries()} />}
      </div>
    )

  return (
    <section className="flex flex-col gap-7">
      <PageHead title="Meetings"
        lede={data ? counts(data, provider) : undefined}
        actions={<>
          <Link to="/upload" className="inline-flex items-center gap-2 rounded-md border border-hair bg-sheet px-3 py-1.5 text-sm font-medium hover:border-ink-2">Upload</Link>
          <Button onClick={() => setSending(true)}>Send notetaker</Button>
          <Button variant="primary" onClick={() => setRecording(true)}>Record from this browser</Button>
        </>} />
      {sending && <SendNotetaker onClose={() => setSending(false)} />}
      {recording && <RecordDialog onClose={() => setRecording(false)} />}
      {data?.filter((m) => m.status === 'recording').map((m) => <LiveCard key={m.id} m={m} />)}
      <Columns>
      <div className="frame @container flex min-w-0 flex-col gap-7">
      {isLoading && <p className="text-mute">Loading…</p>}
      {error && !(error instanceof Unauthorized) && <p role="alert" className="text-bad">Could not load meetings.</p>}
      {data?.length === 0 && <p className="text-mute">No meetings yet. <Link to="/upload" className="text-primary">Upload a recording</Link> or send the notetaker to a call.</p>}
      {data && groupByDay(data).map((g) => (
        <div key={g.label} className="flex flex-col">
          <h2 className="pb-2 text-xs font-semibold tracking-[0.08em] text-mute uppercase">{g.label}</h2>
          <ul>
            {g.items.map((m) => (
              <li key={m.id} className="grid grid-cols-[64px_minmax(0,1fr)_120px_auto] items-center gap-4 rounded border-t border-hair px-1.5 py-3 hover:bg-hair-2 @max-xl:grid-cols-[minmax(0,1fr)_auto]">
                <span className="font-mono text-xs text-mute @max-xl:hidden">{at(meetingTime(m))}</span>
                <div className="min-w-0">
                  <Link to={`/meetings/${m.id}`} className="block truncate font-semibold tracking-tight hover:text-primary">{m.title}</Link>
                  <p className="flex flex-wrap gap-x-2.5 text-xs text-mute"><span className="font-mono @xl:hidden">{at(meetingTime(m))}</span>
                    {m.duration_ms ? <span>{minutesLabel(m.duration_ms)}</span> : null}<span>{m.platform}</span>
                    {m.action_item_count ? <span>{m.action_item_count} action {m.action_item_count === 1 ? 'item' : 'items'}</span> : null}</p>
                </div>
                <span className="@max-xl:col-span-full @max-xl:row-start-2 @max-xl:empty:hidden">
                  {m.speakers?.length ? <Ribbon segments={m.speakers.map((_, i) => ({ speaker: i, weight: 1 }))} /> : null}
                </span>
                <div className="flex shrink-0 items-center gap-3">
                  {m.speakers?.length ? <span className="flex @max-xs:hidden">{m.speakers.map((p, i) => <span key={p.id} className={i ? '-ml-1.5' : ''}><Face name={p.name} speaker={i} /></span>)}</span> : null}
                  {Object.hasOwn(statusBadge, m.status)
                    ? <Badge tone={statusBadge[m.status].tone}>{statusBadge[m.status].label}</Badge>
                    : <Badge>{m.status}</Badge>}
                  <Button variant="danger" aria-label={`Delete ${m.title}`} onClick={() => { del.reset(); setDeleting(m) }}>Delete</Button>
                </div>
              </li>
            ))}
          </ul>
        </div>
      ))}
      </div>
      <NextUp />
      </Columns>
      {deleting && (
        <ConfirmDialog title="Delete this meeting?" confirmLabel="Delete meeting" busy={del.isPending}
          error={del.error ? del.error.message.replace(/^\d+ /, '') : undefined}
          onConfirm={() => del.mutate(deleting.id)} onCancel={() => setDeleting(null)}>
          “{deleting.title}”: the audio, transcript, report and Ask search entries are removed permanently. This cannot be undone.
        </ConfirmDialog>
      )}
    </section>
  )
}
