import type { ReactNode } from 'react'

export function PageHead({ title, lede, actions }: { title: string; lede?: string; actions?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-x-5 gap-y-3">
      <div>
        <h1 className="text-[1.6rem] leading-tight font-semibold tracking-tight">{title}</h1>
        {lede && <p className="mt-1.5 max-w-[60ch] text-mute">{lede}</p>}
      </div>
      {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
    </div>
  )
}
