import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router'
import { getMeetings, Unauthorized, uploadFile } from '../api'

const when = (ms: number) => new Date(ms).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })

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

  if (error instanceof Unauthorized)
    return (
      <div className="rounded-xl border border-hair bg-sheet p-8">
        <h1 className="text-xl font-semibold">Sign in to Recapz</h1>
        <p className="mt-2 text-mute">Access is by invitation. Use the Google account you were invited with.</p>
        <a href="/auth/google" className="mt-4 inline-block rounded-lg bg-primary px-4 py-2 font-medium text-white">
          Sign in with Google
        </a>
      </div>
    )

  return (
    <section>
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold tracking-tight">Meetings</h1>
        <label className="cursor-pointer rounded-lg bg-primary px-4 py-2 text-sm font-medium text-white">
          {upload.isPending ? 'Uploading…' : 'Upload recording'}
          <input
            type="file"
            accept="audio/*,video/*"
            className="sr-only"
            onChange={(e) => e.target.files?.[0] && upload.mutate(e.target.files[0])}
          />
        </label>
      </div>
      {upload.isError && <p className="mt-3 text-sm text-red-700">Upload failed. Try again.</p>}
      {isLoading && <p className="mt-6 text-mute">Loading…</p>}
      {error && !(error instanceof Unauthorized) && <p className="mt-6 text-red-700">Could not load meetings.</p>}
      {data?.length === 0 && <p className="mt-6 text-mute">No meetings yet. Upload a recording to get started.</p>}
      <ul className="mt-6 divide-y divide-hair overflow-hidden rounded-xl border border-hair bg-sheet">
        {data?.map((m) => (
          <li key={m.id} className="flex items-center justify-between gap-4 px-5 py-4">
            <div className="min-w-0">
              <Link to={`/meetings/${m.id}`} className="block truncate font-medium hover:text-primary">{m.title}</Link>
              <p className="font-mono text-xs text-mute">{when(m.started_at ?? m.created_at)} · {m.platform}</p>
            </div>
            <span className="rounded-full border border-hair px-2.5 py-0.5 font-mono text-xs text-mute">{m.status}</span>
          </li>
        ))}
      </ul>
    </section>
  )
}
