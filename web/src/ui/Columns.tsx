import type { ReactNode } from 'react'

/**
 * Page body: a main column and a 320 px side column. It stacks when the page area (the `@container` on <main>),
 * not the window, is under 56rem, so the rail's width is already accounted for.
 */
export const Columns = ({ children }: { children: ReactNode }) => (
  <div className="grid grid-cols-[minmax(0,1fr)_320px] items-start gap-9 @max-4xl:grid-cols-1 @max-4xl:gap-7">{children}</div>
)
