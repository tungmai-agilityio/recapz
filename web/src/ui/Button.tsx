import type { ButtonHTMLAttributes } from 'react'

const base = 'inline-flex items-center gap-2 rounded-md border px-3 py-1.5 text-sm font-medium whitespace-nowrap disabled:opacity-50'
const variants = {
  default: 'border-hair bg-sheet text-ink hover:border-ink-2',
  primary: 'border-primary bg-primary text-primary-fg hover:opacity-90',
  ghost: 'border-transparent bg-transparent text-ink hover:bg-hair-2',
  danger: 'border-hair bg-sheet text-bad hover:border-bad',
}

export function Button({ variant = 'default', className = '', type = 'button', ...rest }:
  ButtonHTMLAttributes<HTMLButtonElement> & { variant?: keyof typeof variants }) {
  return <button type={type} className={`${base} ${variants[variant]} ${className}`} {...rest} />
}
