import { useQuery } from '@tanstack/react-query'
import { Link, useParams } from 'react-router'
import { clock } from '../clock'
import { getMeeting, getTranscript, Unauthorized } from '../api'

export function Report() {
  const { id = '' } = useParams()
  const meeting = useQuery({ queryKey: ['meeting', id], queryFn: () => getMeeting(id) })
  const transcript = useQuery({
    queryKey: ['transcript', id],
    queryFn: () => getTranscript(id),
    enabled: meeting.isSuccess,
  })

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

      <h2 className="mt-8 text-lg font-semibold">Transcript</h2>
      {transcript.isLoading && <p className="mt-3 text-mute">Loading…</p>}
      {transcript.isError && <p className="mt-3 text-red-700">Could not load the transcript.</p>}
      {transcript.data?.length === 0 && (
        <p className="mt-3 text-mute">
          {m.status === 'ready' ? 'No speech was found.' : 'The transcript will appear when processing finishes.'}
        </p>
      )}
      <ol className="mt-3 space-y-3">
        {transcript.data?.map((s) => (
          <li key={s.id} className="flex gap-4">
            <time className="w-14 shrink-0 pt-0.5 font-mono text-xs text-mute">{clock(s.start_ms)}</time>
            <p>{s.text}</p>
          </li>
        ))}
      </ol>
    </article>
  )
}
