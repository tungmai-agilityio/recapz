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

/** Echoes the csrf cookie in a header on state-changing requests. */
const csrf = () => ({ 'x-csrf-token': /(?:^|; )recapz_csrf=([^;]*)/.exec(document.cookie)?.[1] ?? '' })

async function check(res: Response) {
  if (res.status === 401) throw new Unauthorized()
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`)
  return res
}

export const getMeetings = async (): Promise<Meeting[]> =>
  (await check(await fetch('/api/meetings'))).json()

export const getMeeting = async (id: string): Promise<MeetingDetail> =>
  (await check(await fetch(`/api/meetings/${id}`))).json()

export const getTranscript = async (id: string): Promise<Segment[]> =>
  (await check(await fetch(`/api/meetings/${id}/transcript`))).json()

/** `null` until the report has been written. */
export const getReport = async (id: string): Promise<ReportData | null> =>
  (await check(await fetch(`/api/meetings/${id}/report`))).json()

export async function uploadFile(file: File) {
  const body = new FormData()
  body.append('file', file)
  await check(await fetch('/api/uploads', { method: 'POST', body, headers: csrf() }))
}

export const logout = async () => { await fetch('/auth/logout', { method: 'POST', headers: csrf() }) }

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
