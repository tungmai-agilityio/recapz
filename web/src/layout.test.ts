import { describe, expect, it } from 'vitest'

// Page content sits beside the rail and often in a column, so the window width says little about the room it has.
// Content reacts to its container (`@container` + `@max-*:`); only Shell may use window breakpoints.
const files = import.meta.glob(['./pages/*.tsx', './ui/*.tsx', '!./**/*.test.tsx'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>
const viewport = /(?<![@\w-])(?:(?:sm|md|lg|xl|2xl):|(?:max|min)-\[\d+px\]:)/

describe('layout', () => {
  it('uses container queries, not window breakpoints, outside Shell', () => {
    expect(Object.keys(files).length).toBeGreaterThan(20)
    const offenders = Object.entries(files).filter(([, src]) => viewport.test(src)).map(([path]) => path)
    expect(offenders).toEqual([])
  })
  it('flags a window breakpoint', () => {
    expect(viewport.test('className="grid sm:grid-cols-3"')).toBe(true)
    expect(viewport.test('className="max-[900px]:hidden"')).toBe(true)
    expect(viewport.test('className="@sm:grid-cols-3 @max-xl:hidden max-w-[60ch]"')).toBe(false)
  })
})
