import type { ButtonHTMLAttributes, Ref } from 'react'
import { buttonClass, type Variant } from './buttonClass'

export function Button({ variant = 'default', className = '', type = 'button', ...rest }:
  ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; ref?: Ref<HTMLButtonElement> }) {
  return <button type={type} className={`${buttonClass(variant)} ${className}`} {...rest} />
}
