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
  // 4800 input samples = 100 ms = one frame, but the filter runs 72 samples behind: 4992 (39 quanta) gives one.
  const frames = run(48000, Array.from({ length: 39 }, () => [constant(0.25)]))
  expect(frames).toHaveLength(1)
  expect(frames[0].pcm.byteLength).toBe(3200)
  expect(samples(frames[0])).toEqual(Array(FRAME_SAMPLES).fill(8192)) // 0.25 * 32767 = 8191.75; 0.5 would sit on a rounding boundary the filter's last bit can flip
  const bytes = new Uint8Array(frames[0].pcm)
  expect([bytes[0], bytes[1]]).toEqual([0x00, 0x20]) // 8192 = 0x2000, low byte first
})

test('44.1 kHz gives one frame per 4410 input samples', () => {
  const frames = run(44100, [[constant(-0.25, 4410 * 3 + 100)]])
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

const tone = (rate: number, hz: number) =>
  Float32Array.from({ length: rate }, (_, n) => 0.5 * Math.sin((2 * Math.PI * hz * n) / rate))
const rms = (rate: number, hz: number) => {
  const input = tone(rate, hz)
  const quanta = Array.from({ length: Math.floor(rate / 128) }, (_, i) => [input.subarray(i * 128, (i + 1) * 128)])
  const out = run(rate, quanta).flatMap(samples).map((v) => v / 32767).slice(100, -100)
  return Math.sqrt(out.reduce((a, v) => a + v * v, 0) / out.length)
}

test('speech-band tones pass and content above 8 kHz is removed, not folded back', () => {
  // A 0.5 tone has RMS 0.3536. The old box filter turned a 10 kHz tone into a 6 kHz one at half amplitude.
  for (const rate of [48000, 44100]) {
    for (const hz of [300, 1000, 4000, 6000]) expect(rms(rate, hz)).toBeCloseTo(0.3536, 2)
    for (const hz of [10000, 12000, 20000]) expect(rms(rate, hz)).toBeLessThan(0.0035)
  }
})
