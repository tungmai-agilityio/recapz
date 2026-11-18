import type { ReactNode } from 'react'

/** Plain data table: `head` is the column titles; rows are `Row`s of `Cell`s. */
export function Table({ label, head, children }: { label: string; head: string[]; children: ReactNode }) {
  return (
    <div className="overflow-x-auto">
      <table aria-label={label} className="w-full border-collapse text-left text-sm">
        <thead>
          <tr className="border-b border-hair text-xs tracking-wide text-mute uppercase">
            {head.map((h, i) => <th key={i} scope="col" className="px-3 py-2 font-medium">{h}</th>)}
          </tr>
        </thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  )
}

export const Row = ({ children }: { children: ReactNode }) => <tr className="border-b border-hair-2 last:border-0">{children}</tr>
export const Cell = ({ children, className = '' }: { children: ReactNode; className?: string }) => <td className={`px-3 py-2.5 align-middle ${className}`}>{children}</td>
