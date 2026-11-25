/** Samples per frame: 100 ms at 16 kHz. */
export const FRAME_SAMPLES = 1600
export const TARGET_RATE = 16000

export type Frame = { pcm: ArrayBuffer; levels: number[] }

const CUTOFF = 0.45 // of the output rate: 7.2 kHz
const HALF_OUT = 24 // filter half-width in output samples
const PHASES = 256 // fractional positions kept; rates whose reduced ratio needs fewer are exact

const gcd = (a: number, b: number): number => (b === 0 ? a : gcd(b, a % b))

/**
 * Streaming polyphase windowed-sinc (Blackman) low-pass and decimator: flat to about 6 kHz, down more than
 * 60 dB from 9 kHz, so 8 to 16 kHz content cannot fold into the speech band. Output k sits at input position
 * k * from / to, tracked in integers. It runs `half` input samples behind and pads the start with the first sample.
 */
class Resampler {
  private readonly f: number
  private readonly t: number
  private readonly half: number
  private readonly phases: Float64Array[]
  private buf: number[] = []
  private c: number
  private ph = 0

  constructor(from: number, to: number) {
    const g = gcd(from, to)
    this.f = from / g
    this.t = to / g
    const ratio = from / to
    this.half = Math.ceil(HALF_OUT * Math.max(ratio, 1))
    this.c = this.half
    const wc = CUTOFF / Math.max(ratio, 1)
    const count = Math.min(this.t, PHASES)
    this.phases = Array.from({ length: count }, (_, p) => {
      const taps = Float64Array.from({ length: 2 * this.half }, (_, i) => {
        const u = i + 1 - this.half - p / count
        const x = 2 * wc * u
        const sinc = x === 0 ? 1 : Math.sin(Math.PI * x) / (Math.PI * x)
        const a = (Math.PI * u) / this.half
        return sinc * (0.42 + 0.5 * Math.cos(a) + 0.08 * Math.cos(2 * a))
      })
      const sum = taps.reduce((a, b) => a + b, 0)
      return taps.map((v) => v / sum) // sums to 1: a constant keeps its level
    })
  }

  push(x: number, out: (y: number) => void) {
    if (this.buf.length === 0) this.buf = Array(this.half).fill(x)
    this.buf.push(x)
    while (this.c + this.half < this.buf.length) {
      const taps = this.phases[Math.floor((this.ph * this.phases.length) / this.t)]
      const start = this.c + 1 - this.half
      let y = 0
      for (let i = 0; i < taps.length; i++) y += taps[i] * this.buf[start + i]
      out(y)
      this.ph += this.f
      this.c += Math.floor(this.ph / this.t)
      this.ph %= this.t
    }
    if (this.c > 4 * this.half) {
      const drop = this.c + 1 - this.half
      this.buf.splice(0, drop)
      this.c -= drop
    }
  }
}

/**
 * Mixes mono inputs, resamples them to 16 kHz and cuts 1600-sample frames of little-endian i16.
 * Pure, so it runs in the AudioWorklet and in tests alike.
 */
export class Framer {
  /** Undefined at 16 kHz: the capture asks the browser for a 16 kHz context, so this only runs as a fallback. */
  private readonly resampler?: Resampler
  private readonly onFrame: (frame: Frame) => void
  private out = new DataView(new ArrayBuffer(FRAME_SAMPLES * 2))
  private filled = 0
  private peaks: number[] = []
  private readonly emitOut = (y: number) => this.emit(y)

  constructor(inputRate: number, onFrame: (frame: Frame) => void) {
    if (Math.round(inputRate) !== TARGET_RATE) this.resampler = new Resampler(Math.round(inputRate), TARGET_RATE)
    this.onFrame = onFrame
  }

  /** One render quantum: `inputs[i]` is input i's mono samples, or undefined when nothing is connected. */
  push(inputs: (Float32Array | undefined)[]) {
    const len = Math.max(0, ...inputs.map((c) => c?.length ?? 0))
    inputs.forEach((c, i) => {
      let peak = this.peaks[i] ?? 0
      for (const s of c ?? []) peak = Math.max(peak, Math.abs(s))
      this.peaks[i] = peak
    })
    for (let n = 0; n < len; n++) {
      let x = 0
      for (const c of inputs) x += c?.[n] ?? 0
      if (this.resampler) this.resampler.push(x, this.emitOut)
      else this.emit(x)
    }
  }

  private emit(x: number) {
    const v = Math.max(-1, Math.min(1, x))
    this.out.setInt16(this.filled * 2, Math.round(v * 32767), true)
    if (++this.filled < FRAME_SAMPLES) return
    this.onFrame({ pcm: this.out.buffer as ArrayBuffer, levels: this.peaks.map((p) => Math.min(1, p)) })
    this.out = new DataView(new ArrayBuffer(FRAME_SAMPLES * 2))
    this.filled = 0
    this.peaks = []
  }
}
