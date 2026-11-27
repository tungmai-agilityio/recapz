/** What the server's config says about where data lives, as the sentences the privacy page shows. No provider is named. */
export function whereDataLives(c: { where: 'object_storage' | 'local_disk'; region?: string; days?: number; meetingDays?: number }): string {
  const parts = [
    c.where === 'object_storage'
      ? `Finished recordings are kept in private object storage${c.region ? ` in region ${c.region}` : ''}.`
      : "Finished recordings are kept on this server's disk.",
    "Backups are written to this server's disk.",
  ]
  if (c.days !== undefined) parts.push(c.days === 0 ? 'Audio is kept until someone deletes the meeting.' : `Audio is deleted after ${c.days} days.`)
  parts.push(c.meetingDays ? `Transcripts and reports are deleted with the meeting after ${c.meetingDays === 365 ? '1 year' : `${c.meetingDays} days`}.` : 'Transcripts and reports stay.')
  return parts.join(' ')
}
