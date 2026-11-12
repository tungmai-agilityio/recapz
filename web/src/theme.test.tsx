import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { ThemeSwitch } from './ThemeSwitch'
import { applyTheme, initTheme } from './theme'

let systemDark = false
let onChange: () => void = () => {}
const html = document.documentElement

beforeEach(() => {
  localStorage.clear()
  systemDark = false
  vi.stubGlobal('matchMedia', () => ({ matches: systemDark, addEventListener: (_: string, f: () => void) => { onChange = f } }))
})
afterEach(() => { cleanup(); vi.unstubAllGlobals() })

test('defaults to Daylight and light', () => {
  applyTheme()
  expect(html.dataset.skin).toBe('day')
  expect(html.dataset.theme).toBe('light')
})

test('stored Ink and dark are applied', () => {
  localStorage.setItem('recapz-skin', 'ink')
  localStorage.setItem('recapz-mode', 'dark')
  applyTheme()
  expect(html.dataset.skin).toBe('ink')
  expect(html.dataset.theme).toBe('dark')
})

test('Auto follows the system preference', () => {
  localStorage.setItem('recapz-mode', 'system')
  systemDark = true
  applyTheme()
  expect(html.dataset.theme).toBe('dark')
  systemDark = false
  applyTheme()
  expect(html.dataset.theme).toBe('light')
})

test('the switch changes skin and mode, persists them and marks the pressed button', () => {
  render(<ThemeSwitch />)
  const ink = screen.getByRole('button', { name: 'Ink' })
  expect(screen.getByRole('button', { name: 'Daylight' }).getAttribute('aria-pressed')).toBe('true')
  fireEvent.click(ink)
  fireEvent.click(screen.getByRole('button', { name: /Dark/ }))
  expect(ink.getAttribute('aria-pressed')).toBe('true')
  expect(html.dataset.skin).toBe('ink')
  expect(html.dataset.theme).toBe('dark')
  expect(localStorage.getItem('recapz-skin')).toBe('ink')
  expect(localStorage.getItem('recapz-mode')).toBe('dark')
})

test('invalid stored values fall back to Daylight and light', () => {
  localStorage.setItem('recapz-skin', 'foo')
  localStorage.setItem('recapz-mode', 'bar')
  applyTheme()
  expect(html.dataset.skin).toBe('day')
  expect(html.dataset.theme).toBe('light')
})

test('Auto re-applies when the system preference changes', () => {
  localStorage.setItem('recapz-mode', 'system')
  initTheme()
  expect(html.dataset.theme).toBe('light')
  systemDark = true
  onChange()
  expect(html.dataset.theme).toBe('dark')
})
