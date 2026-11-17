const colours = ['bg-speaker-1', 'bg-speaker-2', 'bg-speaker-3', 'bg-speaker-4', 'bg-speaker-5']

/** Tailwind background class for a 0-based speaker index (null = unknown); one person, one colour in ribbon, transcript, faces and talk time. */
export const colour = (s: number | null) => (s === null ? 'bg-hair' : colours[s % colours.length])

/** Colour index of a speaker: its place in the meeting's `speakers` array, the one source every screen uses; null (grey) when absent. */
export const speakerIndex = (speakers: { id: string }[] | undefined, id: string | null) => {
  const i = id === null ? -1 : (speakers ?? []).findIndex((p) => p.id === id)
  return i < 0 ? null : i
}
