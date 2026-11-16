import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router'
import { useState } from 'react'
import { ConfirmDialog } from '../ui/ConfirmDialog'
import { Badge } from '../ui/Badge'
import { Button } from '../ui/Button'
import { PageHead } from '../ui/PageHead'
import { useToast } from '../ui/toast-context'
import { NextUp } from './NextUp'
import { groupByDay, meetingTime, statusBadge } from './meeting-days'
import { deleteMeeting, devLogin, getMeetings, Unauthorized, uploadFile } from '../api'

const at = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
const thisMonth = (ms: number) => { const a = new Date(ms), b = new Date(); return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() }

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

export function Meetings() {
  const qc = useQueryClient()
  const { data, error, isLoading } = useQuery({
    queryKey: ['meetings'],
    queryFn: getMeetings,
    retry: (n, e) => !(e instanceof Unauthorized) && n < 2,
  })
  const upload = useMutation({
    mutationFn: uploadFile,
    onSuccess: () => qc.invalidateQueries({ queryKey: ['meetings'] }),
  })
  const toast = useToast()
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
        {import.meta.env.DEV && <DevLogin onDone={() => qc.invalidateQueries({ queryKey: ['meetings'] })} />}
      </div>
    )

  return (
    <section className="flex flex-col gap-7">
      <PageHead title="Meetings"
        lede={data ? `${data.filter((m) => thisMonth(meetingTime(m))).length} meetings this month · all audio stays on your server` : undefined}
        actions={
          <label className="inline-flex cursor-pointer items-center gap-2 rounded-md border border-primary bg-primary px-3 py-1.5 text-sm font-medium text-primary-fg hover:opacity-90">
            {upload.isPending ? 'Uploading…' : 'Upload recording'}
            <input
              type="file"
              accept="audio/*,video/*"
              className="sr-only"
              onChange={(e) => e.target.files?.[0] && upload.mutate(e.target.files[0])}
            />
          </label>
        } />
      <NextUp />
      {upload.isError && <p className="text-sm text-bad">Upload failed. Try again.</p>}
      {isLoading && <p className="text-mute">Loading…</p>}
      {error && !(error instanceof Unauthorized) && <p className="text-bad">Could not load meetings.</p>}
      {data?.length === 0 && <p className="text-mute">No meetings yet. Upload a recording to get started.</p>}
      {data && groupByDay(data).map((g) => (
        <div key={g.label} className="flex flex-col">
          <h2 className="pb-2 text-xs font-semibold tracking-[0.08em] text-mute uppercase">{g.label}</h2>
          <ul>
            {g.items.map((m) => (
              <li key={m.id} className="grid grid-cols-[64px_minmax(0,1fr)_auto] items-center gap-4 rounded border-t border-hair px-1.5 py-3 hover:bg-hair-2 max-[900px]:grid-cols-[minmax(0,1fr)_auto]">
                <span className="font-mono text-xs text-mute max-[900px]:hidden">{at(meetingTime(m))}</span>
                <div className="min-w-0">
                  <Link to={`/meetings/${m.id}`} className="block truncate font-semibold tracking-tight hover:text-primary">{m.title}</Link>
                  <p className="flex gap-2.5 text-xs text-mute"><span className="font-mono min-[901px]:hidden">{at(meetingTime(m))}</span><span>{m.platform}</span></p>
                </div>
                <div className="flex shrink-0 items-center gap-3">
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
