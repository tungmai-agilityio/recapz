export interface Meeting {
  id: string
  title: string
  platform: string
  status: string
  started_at: number | null
  created_at: number
  duration_ms?: number | null
  action_item_count?: number
  speakers?: { id: string; name: string }[]
}

export interface MeetingDetail extends Meeting {
  language: string
  ended_at: number | null
  error: string | null
  share: MeetingShare
  /** The owner or an admin: may rename, re-share and delete. */
  can_edit: boolean
}

export type MeetingShare = 'attendees' | 'workspace' | 'owner'

export interface Segment {
  id: number
  speaker_id: string | null
  speaker_name: string | null
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

export interface Me { email: string; name: string | null; role: string; workspace?: { id: string; name: string }; recording_storage?: 'object_storage' | 'local_disk' }

/** Echoes the csrf cookie in a header on state-changing requests. */
const csrf = () => ({ 'x-csrf-token': /(?:^|; )recapz_csrf=([^;]*)/.exec(document.cookie)?.[1] ?? '' })

async function check(res: Response) {
  if (res.status === 401) throw new Unauthorized()
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`)
  return res
}

export const getMe = async (): Promise<Me> => (await check(await fetch('/api/me'))).json()

export const getAudioRetention = async (): Promise<{ days: number }> =>
  (await check(await fetch('/api/settings/audio-retention'))).json()

/** Admin only; the server refuses more than 3650 days. */
export const putAudioRetention = async (days: number): Promise<{ days: number }> =>
  (await check(await fetch('/api/settings/audio-retention', {
    method: 'PUT', headers: { 'content-type': 'application/json', ...csrf() }, body: JSON.stringify({ days }),
  }))).json()

export interface JoinSettings { join_rule: 'manual' | 'auto'; notetaker_name: string; join_message: string }

export const getJoinSettings = async (): Promise<JoinSettings> =>
  (await check(await fetch('/api/settings'))).json()

/** Admin only. Replaces all three values; a bad one is a 400 with the reason as text. */
export const putJoinSettings = async (s: JoinSettings): Promise<JoinSettings> =>
  (await check(await fetch('/api/settings', {
    method: 'PUT', headers: { 'content-type': 'application/json', ...csrf() }, body: JSON.stringify(s),
  }))).json()

export interface Usage {
  speech_micro_usd: number
  llm_micro_usd: number
  total_micro_usd: number
  cap_micro_usd: number
  paused: boolean
  meetings: { id: string; title: string; speech_micro_usd: number; llm_micro_usd: number }[]
}

/** Admin only: this month's estimated cost, the cap and the costliest meetings. */
export const getUsage = async (): Promise<Usage> => (await check(await fetch('/api/usage'))).json()

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

/** Owner or admin only: the server answers 403 for anyone else who can see the meeting. */
export const setMeetingShare = async (id: string, share: MeetingShare): Promise<{ share: MeetingShare }> =>
  (await check(await fetch(`/api/meetings/${id}`, { method: 'PATCH', headers: { ...csrf(), 'content-type': 'application/json' }, body: JSON.stringify({ share }) }))).json()

/** Only before the notetaker joins; the server answers 409 afterwards and 403 for a non-owner. */
export const setMeetingLanguage = async (id: string, language: MeetingLanguage): Promise<{ language: MeetingLanguage }> =>
  (await check(await fetch(`/api/meetings/${id}/language`, { method: 'PATCH', headers: { ...csrf(), 'content-type': 'application/json' }, body: JSON.stringify({ language }) }))).json()

/** Queues a fresh report; the server answers 409 unless the meeting is ready and 403 for a non-owner. */
export const regenerateReport = async (id: string) => { await check(await fetch(`/api/meetings/${id}/report/regenerate`, { method: 'POST', headers: csrf() })) }

export const deleteMeeting = async (id: string) => { await check(await fetch(`/api/meetings/${id}`, { method: 'DELETE', headers: csrf() })) }

/** Sends the notetaker to a Meet or Zoom link; the server answers 400 for any other link. */
export const joinMeeting = async (url: string, title: string) => {
  await check(await fetch('/api/meetings/join', {
    method: 'POST', headers: { 'content-type': 'application/json', ...csrf() }, body: JSON.stringify({ url, title: title || undefined }),
  }))
}

/** Returns the new meeting's id. The server also reads an optional `title` (default: the file name). */
/** The speech language of a meeting: Vietnamese (default), English, or mixed. */
export type MeetingLanguage = 'vi' | 'en' | 'multi'

export async function uploadRecording(file: File, title?: string, language: MeetingLanguage = 'vi'): Promise<{ id: string }> {
  const body = new FormData()
  if (title?.trim()) body.append('title', title.trim())
  body.append('language', language)
  body.append('file', file)
  return (await check(await fetch('/api/uploads', { method: 'POST', body, headers: csrf() }))).json()
}

export const uploadFile = (file: File) => uploadRecording(file)

/** Starts a browser recording: the server creates the meeting already recording and waits for its socket. */
export const startRecording = async (title: string, language: MeetingLanguage, mode: 'tab' | 'room'): Promise<{ id: string }> =>
  (await check(await fetch('/api/recordings', {
    method: 'POST', headers: { 'content-type': 'application/json', ...csrf() }, body: JSON.stringify({ title, language, mode }),
  }))).json()

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
  /** Exactly one of `meeting_id` and `document_id` is set. */
  meeting_id: string | null
  document_id: string | null
  /** The meeting title or the document name. */
  title: string
  /** Moment in the meeting; null for a document. */
  start_ms: number | null
  quote: string
  /** False for a passage from a document not allowed to go to the language model: no answer used it. */
  answered: boolean
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

/** Ask refused for now: 429 (too many questions) or 503 (no language model configured). */
export class RateLimited extends Error {}
export class NoModel extends Error {}
/** The stream ended with an `error` event after it had started. */
export class AnswerFailed extends Error {}

/** Streams an answer: `onToken` gets each piece of text, the result is the cited moments. */
export async function ask(question: string, onToken: (t: string) => void): Promise<Citation[]> {
  const first = await fetch('/api/ask', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', ...csrf() },
    body: JSON.stringify({ question }),
  })
  if (first.status === 429) throw new RateLimited()
  if (first.status === 503) throw new NoModel()
  const res = await check(first)
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
      if (e.name === 'error') throw new AnswerFailed()
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

export interface Doc { id: string; name: string; llm_ok: boolean; status: 'indexing' | 'indexed' | 'no text found'; size_bytes: number; /** Upload time, ms since the epoch. */ created_at: number }

/** What the member has stored against the per-member cap. */
export interface DocumentUsage { used_bytes: number; cap_bytes: number }

export const getDocumentUsage = async (): Promise<DocumentUsage> => (await check(await fetch('/api/documents/usage'))).json()

export const getDocuments = async (): Promise<Doc[]> => (await check(await fetch('/api/documents'))).json()

export async function uploadDocument(file: File): Promise<{ id: string }> {
  const body = new FormData()
  body.append('file', file)
  return (await check(await fetch('/api/documents', { method: 'POST', body, headers: csrf() }))).json()
}

export const setDocumentLlmOk = async (id: string, llm_ok: boolean) => {
  await check(await fetch(`/api/documents/${id}`, {
    method: 'PATCH', headers: { 'content-type': 'application/json', ...csrf() }, body: JSON.stringify({ llm_ok }),
  }))
}

export const deleteDocument = async (id: string) => { await check(await fetch(`/api/documents/${id}`, { method: 'DELETE', headers: csrf() })) }

export type LiveEvent =
  | { type: 'final'; at_ms: number; text: string }
  | { type: 'partial'; text: string }
  | { type: 'unavailable' }
  | { type: 'end' }

/** The meeting is not being recorded right now, or the member cannot see it. */
export class NotLive extends Error {}

/** Reads the live transcript until the call ends, the stream drops or `signal` aborts. */
export async function watchLive(id: string, onEvent: (e: LiveEvent) => void, signal: AbortSignal): Promise<void> {
  const first = await fetch(`/api/meetings/${id}/live`, { signal })
  if (first.status === 404) throw new NotLive()
  const res = await check(first)
  const reader = res.body!.getReader()
  const decoder = new TextDecoder()
  let buf = ''
  for (;;) {
    const { done, value } = await reader.read()
    buf += decoder.decode(value, { stream: !done })
    const { events, rest } = parseEvents(done ? buf + '\n\n' : buf)
    buf = rest
    for (const e of events) onEvent({ type: e.name, ...(e.data as object) } as LiveEvent)
    if (done) return
  }
}

/** What to ask about: a clicked line (its `at_ms`) or, without one, the last 60 s; `text` is extra words. */
export interface LiveAsk { text?: string; at_ms?: number }

/** One answer to a live question; `citations` with `answered: false` are passages only. */
export async function liveAsk(id: string, body: LiveAsk): Promise<{ answer: string; citations: Citation[] }> {
  const first = await fetch(`/api/meetings/${id}/live-ask`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', ...csrf() },
    body: JSON.stringify(body),
  })
  if (first.status === 404) throw new NotLive()
  if (first.status === 429) throw new RateLimited()
  if (first.status === 503) throw new NoModel()
  const res = await check(first)
  let answer = ''
  let citations: Citation[] = []
  for (const e of parseEvents((await res.text()) + '\n\n').events) {
    if (e.name === 'token') answer += (e.data as { text: string }).text
    if (e.name === 'error') throw new AnswerFailed()
    if (e.name === 'citations') citations = e.data as Citation[]
  }
  return { answer, citations }
}

export type Role = 'admin' | 'member'
export interface Member { id: string; email: string; name: string | null; role: Role; meetings: number }
export interface Invite { email: string; role: Role; created_at: number }

/** Admin only (403 otherwise): the members of this workspace and its pending invites. */
export const getMembers = async (): Promise<{ members: Member[]; invites: Invite[] }> =>
  (await check(await fetch('/api/members'))).json()

/** Returns the sign-in link to copy; the invite carries no token. */
export const inviteMember = async (email: string, role: Role): Promise<{ link: string }> =>
  (await check(await fetch('/api/members/invites', {
    method: 'POST', headers: { 'content-type': 'application/json', ...csrf() }, body: JSON.stringify({ email, role }),
  }))).json()

export const cancelInvite = async (email: string) => {
  await check(await fetch(`/api/members/invites/${encodeURIComponent(email)}`, { method: 'DELETE', headers: csrf() }))
}

export const setMemberRole = async (id: string, role: Role) => {
  await check(await fetch(`/api/members/${id}`, {
    method: 'PATCH', headers: { 'content-type': 'application/json', ...csrf() }, body: JSON.stringify({ role }),
  }))
}

/** Their meetings move to the admin who removes them. */
export const removeMember = async (id: string) => { await check(await fetch(`/api/members/${id}`, { method: 'DELETE', headers: csrf() })) }

export interface Workspace { id: string; name: string; role: Role; current: boolean }

/** The workspaces of the signed-in person; also accepts their pending invites. */
export const getWorkspaces = async (): Promise<Workspace[]> => (await check(await fetch('/api/workspaces'))).json()

export const switchWorkspace = async (id: string) => {
  await check(await fetch(`/api/workspaces/${encodeURIComponent(id)}/switch`, { method: 'POST', headers: csrf() }))
}
