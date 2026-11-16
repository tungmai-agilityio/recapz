export interface Meeting {
  id: string
  title: string
  platform: string
  status: string
  started_at: number | null
  created_at: number
}

export interface MeetingDetail extends Meeting {
  language: string
  ended_at: number | null
  error: string | null
}

export interface Segment {
  id: number
  speaker_id: string | null
  start_ms: number
  end_ms: number
  text: string
}

export interface ActionItem {
  id: string
  text: string
  due_date: string | null
  done: boolean
  source_ms: number | null
}

export interface ReportData {
  summary: string
  decisions: string[]
  open_questions: string[]
  chapters: { start_ms: number; title: string }[]
  action_items: ActionItem[]
}

export class Unauthorized extends Error {}

export interface Me { email: string; name: string | null; role: string }

/** Echoes the csrf cookie in a header on state-changing requests. */
const csrf = () => ({ 'x-csrf-token': /(?:^|; )recapz_csrf=([^;]*)/.exec(document.cookie)?.[1] ?? '' })

async function check(res: Response) {
  if (res.status === 401) throw new Unauthorized()
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`)
  return res
}

export const getMe = async (): Promise<Me> => (await check(await fetch('/api/me'))).json()

export const getMeetings = async (): Promise<Meeting[]> =>
  (await check(await fetch('/api/meetings'))).json()

export const getMeeting = async (id: string): Promise<MeetingDetail> =>
  (await check(await fetch(`/api/meetings/${id}`))).json()

export const getTranscript = async (id: string): Promise<Segment[]> =>
  (await check(await fetch(`/api/meetings/${id}/transcript`))).json()

/** `null` until the report has been written. */
export const getReport = async (id: string): Promise<ReportData | null> =>
  (await check(await fetch(`/api/meetings/${id}/report`))).json()

export const renameMeeting = async (id: string, title: string): Promise<{ title: string }> =>
  (await check(await fetch(`/api/meetings/${id}`, { method: 'PATCH', headers: { ...csrf(), 'content-type': 'application/json' }, body: JSON.stringify({ title }) }))).json()

export const deleteMeeting = async (id: string) => { await check(await fetch(`/api/meetings/${id}`, { method: 'DELETE', headers: csrf() })) }

/** Sends the notetaker to a Meet or Zoom link; the server answers 400 for any other link. */
export const joinMeeting = async (url: string, title: string) => {
  await check(await fetch('/api/meetings/join', {
    method: 'POST', headers: { 'content-type': 'application/json', ...csrf() }, body: JSON.stringify({ url, title: title || undefined }),
  }))
}

export async function uploadFile(file: File) {
  const body = new FormData()
  body.append('file', file)
  await check(await fetch('/api/uploads', { method: 'POST', body, headers: csrf() }))
}

/** Local development only: the server accepts it when `dev_login = true`. */
export async function devLogin(email: string) {
  await check(
    await fetch('/auth/dev-login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email }),
    }),
  )
}

export const logout = async () => { await check(await fetch('/auth/logout', { method: 'POST', headers: csrf() })) }

export interface Citation {
  meeting_id: string
  title: string
  start_ms: number
  quote: string
}

/** Pulls `event:`/`data:` pairs out of a server-sent-event text, leaving any unfinished tail. */
export function parseEvents(buf: string): { events: { name: string; data: unknown }[]; rest: string } {
  const parts = buf.split('\n\n')
  const rest = parts.pop() ?? ''
  const events = parts.map((p) => {
    const name = /^event: (.*)$/m.exec(p)?.[1] ?? ''
    const data = JSON.parse(/^data: (.*)$/m.exec(p)?.[1] ?? 'null')
    return { name, data }
  })
  return { events, rest }
}

/** Streams an answer: `onToken` gets each piece of text, the result is the cited moments. */
export async function ask(question: string, onToken: (t: string) => void): Promise<Citation[]> {
  const res = await check(
    await fetch('/api/ask', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', ...csrf() },
      body: JSON.stringify({ question }),
    }),
  )
  const reader = res.body!.getReader()
  const decoder = new TextDecoder()
  let buf = ''
  let citations: Citation[] = []
  for (;;) {
    const { done, value } = await reader.read()
    buf += decoder.decode(value, { stream: !done })
    const { events, rest } = parseEvents(done ? buf + '\n\n' : buf)
    buf = rest
    for (const e of events) {
      if (e.name === 'token') onToken((e.data as { text: string }).text)
      if (e.name === 'citations') citations = e.data as Citation[]
    }
    if (done) return citations
  }
}

export interface CalendarStatus { connected: boolean; needs_reconnect: boolean }

export const getCalendar = async (): Promise<CalendarStatus> => (await check(await fetch('/api/calendar'))).json()

export const disconnectCalendar = async () => { await check(await fetch('/api/calendar', { method: 'DELETE', headers: csrf() })) }

export interface Attendee {
  name: string | null
  email: string
  status: string | null
}

export interface UpcomingEvent {
  id: string
  title: string
  starts_at: number
  ends_at: number
  platform: string | null
  join_url: string | null
  auto_join: boolean
  attendees: Attendee[]
}

export const getUpcoming = async (): Promise<UpcomingEvent[]> => (await check(await fetch('/api/meetings/upcoming'))).json()

export const setAutoJoin = async (id: string, auto_join: boolean) => {
  await check(await fetch(`/api/calendar-events/${id}`, {
    method: 'PATCH', headers: { 'content-type': 'application/json', ...csrf() }, body: JSON.stringify({ auto_join }),
  }))
}
