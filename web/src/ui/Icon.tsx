/** Line icons for the rail and drop zones (24 px grid, stroke follows the text colour). */
const PATHS = {
  meetings: 'M4 6h16v14H4zM4 10h16M8 3v4M16 3v4',
  ask: 'M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14M20 20l-4-4',
  live: 'M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6M6.3 6.3a8 8 0 0 0 0 11.4M17.7 6.3a8 8 0 0 1 0 11.4',
  upload: 'M12 16V4M7 9l5-5 5 5M4 20h16',
  documents: 'M6 3h9l3 3v15H6zM15 3v4h3M9 12h6M9 16h6',
  recording: 'M12 3l8 3v6c0 5-4 8-8 9-4-1-8-4-8-9V6z',
}

export type IconName = keyof typeof PATHS

export const Icon = ({ name, size = 16 }: { name: IconName; size?: number }) => (
  <svg aria-hidden viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" className="shrink-0">
    <path d={PATHS[name]} />
  </svg>
)
