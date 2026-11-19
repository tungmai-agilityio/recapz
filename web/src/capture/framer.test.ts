import { expect, test } from 'vitest'
import { FRAME_SAMPLES, type Frame, Framer } from './framer'

const run = (rate: number, quanta: (Float32Array | undefined)[][]) => {
  const frames: Frame[] = []
  const f = new Framer(rate, (fr) => frames.push(fr))
  quanta.forEach((q) => f.push(q))
  return frames
}
const samples = (fr: Frame) => Array.from({ length: FRAME_SAMPLES }, (_, i) => new DataView(fr.pcm).getInt16(i * 2, true))
const constant = (v: number, n = 128) => new Float32Array(n).fill(v)

test('48 kHz resamples to frames of exactly 1600 little-endian samples', () => {
  // 4800 input samples = 100 ms = one frame; 4864 (38 quanta) gives one frame and a partial one.
  const frames = run(48000, Array.from({ length: 38 }, () => [constant(0.5)]))
  expect(frames).toHaveLength(1)
  expect(frames[0].pcm.byteLength).toBe(3200)
  expect(samples(frames[0])).toEqual(Array(FRAME_SAMPLES).fill(16384))
  const bytes = new Uint8Array(frames[0].pcm)
  expect([bytes[0], bytes[1]]).toEqual([0x00, 0x40]) // 16384 = 0x4000, low byte first
})

test('44.1 kHz gives one frame per 4410 input samples', () => {
  const frames = run(44100, [[constant(-0.25, 4410 * 3 + 10)]])
  expect(frames).toHaveLength(3)
  expect(new Set(frames.flatMap(samples))).toEqual(new Set([Math.round(-0.25 * 32767)]))
})

test('mixes the microphone and the tab by summing, clamped to full scale', () => {
  const [mixed] = run(16000, [[constant(0.25, 1600), constant(0.5, 1600)]])
  expect(new Set(samples(mixed))).toEqual(new Set([Math.round(0.75 * 32767)]))
  const [clipped] = run(16000, [[constant(0.8, 1600), constant(0.8, 1600)]])
  expect(new Set(samples(clipped))).toEqual(new Set([32767]))
})

test('an unconnected tab input counts as silence and reports per-input peaks', () => {
  const [fr] = run(16000, [[constant(0.3, 1600), undefined]])
  expect(new Set(samples(fr))).toEqual(new Set([Math.round(0.3 * 32767)]))
  expect(fr.levels[0]).toBeCloseTo(0.3)
  expect(fr.levels[1]).toBe(0)
})
