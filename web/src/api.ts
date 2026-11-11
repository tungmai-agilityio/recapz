export interface Meeting {
  id: string
  title: string
  platform: string
  status: string
  started_at: number | null
  created_at: number
}

export class Unauthorized extends Error {}

async function check(res: Response) {
  if (res.status === 401) throw new Unauthorized()
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`)
  return res
}

export const getMeetings = async (): Promise<Meeting[]> =>
  (await check(await fetch('/api/meetings'))).json()

export async function uploadFile(file: File) {
  const body = new FormData()
  body.append('file', file)
  await check(await fetch('/api/uploads', { method: 'POST', body }))
}

export const logout = async () => { await fetch('/auth/logout', { method: 'POST' }) }
