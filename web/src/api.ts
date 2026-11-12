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
  await check(await fetch('/api/uploads', { method: 'POST', body }))
}

export const logout = async () => { await fetch('/auth/logout', { method: 'POST' }) }
