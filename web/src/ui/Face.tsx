import { colour } from './speaker-colour'
const initial = (name: string) => name.trim().charAt(0).toUpperCase() || '?'

/** Initial in a circle; `speaker` (0-based) picks one of the five speaker colours. */
export function Face({ name, speaker, size = 22 }: { name: string; speaker: number; size?: number }) {
  return (
    <span title={name} role="img" aria-label={name} style={{ width: size, height: size, fontSize: size * 0.46 }}
      className={`inline-flex flex-none items-center justify-center rounded-full border-2 border-sheet font-semibold text-white ${colour(speaker)}`}>
      {initial(name)}
    </span>
  )
}

/** Overlapping stack of Faces, at most `max` shown plus a "+n" count. */
export function Faces({ people, max = 4 }: { people: string[]; max?: number }) {
  const shown = people.slice(0, max)
  return (
    <span className="flex">
      {shown.map((p, i) => <span key={p + i} className={i ? '-ml-1.5' : ''}><Face name={p} speaker={i} /></span>)}
      {people.length > max && <span className="ml-1 self-center text-xs text-mute">+{people.length - max}</span>}
    </span>
  )
}
