/** Samples per frame: 100 ms at 16 kHz. */
export const FRAME_SAMPLES = 1600
export const TARGET_RATE = 16000

export type Frame = { pcm: ArrayBuffer; levels: number[] }

/**
 * Mixes mono inputs, resamples them to 16 kHz and cuts 1600-sample frames of little-endian i16.
 * Pure, so it runs in the AudioWorklet and in tests alike.
 */
export class Framer {
  private readonly ratio: number
  private readonly onFrame: (frame: Frame) => void
  private acc = 0
  private sum = 0
  private count = 0
  private out = new DataView(new ArrayBuffer(FRAME_SAMPLES * 2))
  private filled = 0
  private peaks: number[] = []

  constructor(inputRate: number, onFrame: (frame: Frame) => void) {
    this.ratio = inputRate / TARGET_RATE
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
      // ponytail: box-filter decimation; a windowed-sinc low-pass if aliasing ever hurts recognition.
      this.sum += x
      this.count++
      this.acc++
      if (this.acc >= this.ratio) {
        this.acc -= this.ratio
        this.emit(this.sum / this.count)
        this.sum = 0
        this.count = 0
      }
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
