import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render } from '@testing-library/react'
import { readFileSync } from 'node:fs'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test } from 'vitest'
import { Upload } from './pages/Upload'

const css = readFileSync('src/index.css', 'utf8')

afterEach(cleanup)

/** Declarations of the first rule whose selector starts with `selector`. */
const block = (selector: string) => {
  const i = css.indexOf(`${selector} {`)
  expect(i).toBeGreaterThanOrEqual(0)
  return css.slice(i, css.indexOf('}', i))
}

test('Daylight (the :root default) frames sections as sheet cards', () => {
  const day = block(':root')
  expect(day).toContain('--frame-bg: var(--sheet)')
  expect(day).toContain('--frame-border: 1px solid var(--hair)')
  expect(day).toContain('--frame-radius: 10px')
  expect(day).toContain('--frame-pad: 18px 20px')
  expect(day).toContain('--frame-shadow: 0 1px 2px #1b25360a')
  expect(day).toContain('--inset: var(--paper)')
})

test('Ink does not frame: sections sit on the paper', () => {
  const ink = block(":root[data-skin='ink']")
  expect(ink).toContain('--frame-bg: transparent')
  expect(ink).toContain('--frame-border: 0 solid transparent')
  expect(ink).toContain('--frame-radius: 0')
  expect(ink).toContain('--frame-pad: 0')
  expect(ink).toContain('--frame-shadow: none')
  expect(ink).toContain('--inset: var(--sheet)')
})

test('.frame reads only the skin variables', () => {
  const rule = block('.frame')
  for (const v of ['bg', 'border', 'radius', 'pad', 'shadow']) expect(rule).toContain(`var(--frame-${v})`)
})

test('Upload frames its form column and its aside', () => {
  const { container } = render(<QueryClientProvider client={new QueryClient()}><MemoryRouter><Upload /></MemoryRouter></QueryClientProvider>)
  const framed = container.querySelectorAll('.frame')
  expect(framed.length).toBe(2)
  expect(framed[1].tagName).toBe('ASIDE')
})

test('Ink separates settings rows with a hairline instead of a card', () => {
  const rule = block(":root[data-skin='ink'] .frame-row")
  expect(rule).toContain('border-top: 1px solid var(--hair)')
  expect(rule).toContain('padding-top: 22px')
})

test('Recording & privacy frames every section with the shared rule, not its own card classes', () => {
  const src = readFileSync('src/pages/Recording.tsx', 'utf8')
  expect(src.match(/className="(?:mt-\d )?frame frame-row/g)?.length ?? 0).toBe(6)
  expect(src).not.toContain('rounded-lg border border-hair bg-sheet')
})
