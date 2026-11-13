import { expect, test } from 'vitest'
import type { Meeting } from '../api'
import { dayLabel, groupByDay } from './meeting-days'

const at = (y: number, mo: number, d: number, h = 12) => new Date(y, mo, d, h).getTime()
const m = (id: string, ms: number): Meeting => ({ id, title: id, platform: 'upload', status: 'ready', started_at: ms, created_at: 0 })
const now = at(2026, 10, 12, 15)

test('labels today and yesterday, otherwise weekday and date', () => {
  expect(dayLabel(at(2026, 10, 12, 1), now)).toBe('Today')
  expect(dayLabel(at(2026, 10, 11, 23), now)).toBe('Yesterday')
  expect(dayLabel(at(2026, 10, 9), now)).toBe(new Date(2026, 10, 9).toLocaleDateString([], { weekday: 'long', day: 'numeric', month: 'long' }))
})

test('groups by day, newest first, and falls back to created_at', () => {
  const rows = [m('old', at(2026, 10, 9)), m('a', at(2026, 10, 12, 9)), m('b', at(2026, 10, 12, 14)),
    { ...m('c', 0), started_at: null, created_at: at(2026, 10, 11) }]
  const g = groupByDay(rows, now)
  expect(g.map((x) => x.label)).toEqual(['Today', 'Yesterday', dayLabel(at(2026, 10, 9), now)])
  expect(g[0].items.map((x) => x.id)).toEqual(['b', 'a'])
  expect(g[1].items.map((x) => x.id)).toEqual(['c'])
  expect(g[2].items.map((x) => x.id)).toEqual(['old'])
})
