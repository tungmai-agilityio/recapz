import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router'
import { getCalendar, getUpcoming, setAutoJoin } from '../api'
import { Faces } from '../ui/Face'
import { Switch } from '../ui/Switch'
import { dayLabel } from './meeting-days'

const SHOWN = 5
const at = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })

/** Calendar meetings still to come, each with its own join switch. */
export function NextUp() {
  const qc = useQueryClient()
  const cal = useQuery({ queryKey: ['calendar'], queryFn: getCalendar })
  const events = useQuery({ queryKey: ['upcoming'], queryFn: getUpcoming, enabled: cal.data?.connected === true })
  const flip = useMutation({
    mutationFn: ({ id, on }: { id: string; on: boolean }) => setAutoJoin(id, on),
    onSettled: () => qc.invalidateQueries({ queryKey: ['upcoming'] }),
  })

  if (cal.isLoading || cal.isError) return null
  const connectHint = <>Connect Google Calendar on <Link to="/recording" className="text-primary">Recording &amp; privacy</Link>.</>
  let body
  if (!cal.data?.connected) body = <p className="text-sm text-mute">Your calendar is not connected. {connectHint}</p>
  else if (events.isError) body = <p role="alert" className="text-sm text-bad">Could not load upcoming meetings.</p>
  else if (!events.data) body = <p className="text-sm text-mute">Loading…</p>
  else if (events.data.length === 0) body = <p className="text-sm text-mute">No upcoming meetings. {connectHint}</p>
  else
    body = (
      <ul>
        {events.data.slice(0, SHOWN).map((e) => (
          <li key={e.id} className="grid grid-cols-[96px_minmax(0,1fr)_auto] items-center gap-4 border-t border-hair px-1.5 py-3 max-[900px]:grid-cols-[minmax(0,1fr)_auto]">
            <span className="font-mono text-xs text-mute max-[900px]:hidden">{dayLabel(e.starts_at)} {at(e.starts_at)}</span>
            <div className="min-w-0">
              <p className="truncate font-semibold tracking-tight">{e.title}</p>
              <p className="flex gap-2.5 text-xs text-mute">
                <span className="font-mono min-[901px]:hidden">{dayLabel(e.starts_at)} {at(e.starts_at)}</span>
                <span>{e.platform ?? 'No platform'}</span>
                {e.join_url?.startsWith('https://') && <a href={e.join_url} target="_blank" rel="noreferrer" className="text-primary">Open link</a>}
              </p>
              {e.attendees.length > 0 && (
                <span className="mt-1 flex" aria-label={`${e.attendees.length} invited`}>
                  <Faces people={e.attendees.map((a) => a.name ?? a.email)} max={4} />
                </span>
              )}
            </div>
            {e.join_url
              ? (
                <span className="flex items-center gap-2">
                  <span aria-hidden className="text-xs text-mute">Auto-join</span>
                  <Switch label={`Auto-join ${e.title}`} checked={e.auto_join} onChange={(on) => flip.mutate({ id: e.id, on })} />
                </span>
              )
              : <span className="text-xs text-mute">No link</span>}
          </li>
        ))}
      </ul>
    )
  return (
    <div className="flex flex-col">
      <h2 className="pb-2 text-xs font-semibold tracking-[0.08em] text-mute uppercase">Next up</h2>
      {flip.isError && <p role="alert" className="pb-2 text-sm text-bad">Could not change that switch. Try again.</p>}
      {body}
    </div>
  )
}
