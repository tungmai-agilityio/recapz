import type { ReactNode } from 'react'

/**
 * Plain data table: `head` is the column titles; rows are `Row`s of `Cell`s.
 * Under 640 px each row becomes a two-column card and a `Cell`'s `label` is shown above its value,
 * so nothing sits behind a sideways scroll.
 */
export function Table({ label, head, children }: { label: string; head: string[]; children: ReactNode }) {
  return (
    <div className="overflow-x-auto">
      <table aria-label={label} className="w-full border-collapse text-left text-sm max-[640px]:block max-[640px]:[&_tbody]:block">
        <thead className="max-[640px]:sr-only">
          <tr className="border-b border-hair text-xs tracking-wide text-mute uppercase">
            {head.map((h, i) => <th key={i} scope="col" className="px-2.5 py-2 font-medium">{h}</th>)}
          </tr>
        </thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  )
}

export const Row = ({ children }: { children: ReactNode }) => (
  <tr className="border-b border-hair-2 last:border-0 max-[640px]:grid max-[640px]:grid-cols-2 max-[640px]:gap-x-3 max-[640px]:gap-y-2.5 max-[640px]:py-3">{children}</tr>
)
export const Cell = ({ children, label, className = '' }: { children: ReactNode; label?: string; className?: string }) => (
  <td data-label={label}
    className={`px-2.5 py-2.5 align-middle max-[640px]:block max-[640px]:p-0 max-[640px]:before:mb-0.5 max-[640px]:before:block max-[640px]:before:text-xs max-[640px]:before:tracking-wide max-[640px]:before:text-mute max-[640px]:before:uppercase max-[640px]:before:content-[attr(data-label)] ${className}`}>{children}</td>
)
