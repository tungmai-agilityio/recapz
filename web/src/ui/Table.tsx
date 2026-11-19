import type { ReactNode } from 'react'

/**
 * Plain data table: `head` is the column titles; rows are `Row`s of `Cell`s.
 * When the table is under 42rem wide each row becomes a two-column card and a `Cell`'s `label` is shown above its value,
 * so nothing sits behind a sideways scroll.
 */
export function Table({ label, head, children }: { label: string; head: string[]; children: ReactNode }) {
  return (
    <div className="@container overflow-x-auto">
      <table aria-label={label} className="w-full border-collapse text-left text-sm @max-2xl:block @max-2xl:[&_tbody]:block">
        <thead className="@max-2xl:sr-only">
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
  <tr className="border-b border-hair-2 last:border-0 @max-2xl:grid @max-2xl:grid-cols-2 @max-2xl:gap-x-3 @max-2xl:gap-y-2.5 @max-2xl:py-3">{children}</tr>
)
export const Cell = ({ children, label, className = '' }: { children: ReactNode; label?: string; className?: string }) => (
  <td data-label={label}
    className={`px-2.5 py-2.5 align-middle @max-2xl:block @max-2xl:p-0 @max-2xl:before:mb-0.5 @max-2xl:before:block @max-2xl:before:text-xs @max-2xl:before:tracking-wide @max-2xl:before:text-mute @max-2xl:before:uppercase @max-2xl:before:content-[attr(data-label)] ${className}`}>{children}</td>
)
