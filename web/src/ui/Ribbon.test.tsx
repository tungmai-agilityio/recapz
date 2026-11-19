import { cleanup, render } from '@testing-library/react'
import { afterEach, expect, test } from 'vitest'
import { Ribbon } from './Ribbon'

afterEach(cleanup)

test('Ribbon draws one weighted, speaker-coloured segment each, cycling after five, with a variant size', () => {
  const { container } = render(<Ribbon variant="scrubber" segments={[{ speaker: 0, weight: 2 }, { speaker: 5, weight: 0.5 }, { speaker: 2, weight: 1 }]} />)
  const parts = container.querySelectorAll<HTMLElement>('[data-speaker]')
  expect(parts.length).toBe(3)
  expect(parts[0].style.flexGrow).toBe('2')
  expect(parts[0].className).toContain('bg-speaker-1')
  expect(parts[1].style.flexGrow).toBe('0.5')
  expect(parts[1].className).toContain('bg-speaker-1')
  expect(parts[2].className).toContain('bg-speaker-3')
  expect((container.firstChild as HTMLElement).className).toContain('h-7')
})

test('Ribbon list and live variants differ in height', () => {
  const a = render(<Ribbon segments={[]} />).container.firstChild as HTMLElement
  const b = render(<Ribbon variant="live" segments={[]} />).container.firstChild as HTMLElement
  expect(a.className).toContain('h-1.5')
  expect(b.className).toContain('h-5')
})

test('Ribbon chapter labels stop where the next chapter starts, in time order', () => {
  const ticks = [{ ms: 12_000, title: 'C' }, { ms: 0, title: 'A' }, { ms: 10_000, title: 'B' }]
  const { container } = render(<Ribbon variant="scrubber" segments={[]} ticks={ticks} now={0} total={100_000} onSeek={() => {}} />)
  const labels = [...container.querySelectorAll<HTMLElement>('[role=slider] > span[title]')]
  expect(labels.length).toBe(3)
  expect(labels.map((l) => l.title)).toEqual(['A', 'B', 'C'])
  expect(labels.map((l) => l.style.left)).toEqual(['0%', '10%', '12%'])
  expect(labels.map((l) => l.style.maxWidth)).toEqual(['min(8rem, 10%)', 'min(8rem, 2%)', 'min(8rem, 88%)'])
})
