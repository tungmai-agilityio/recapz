// Runs in the AudioWorkletGlobalScope, bundled on its own (imported with `?worker&url`).
import { Framer } from './framer'

declare const sampleRate: number
declare function registerProcessor(name: string, ctor: unknown): void
declare class AudioWorkletProcessor {
  readonly port: MessagePort
}

class PcmProcessor extends AudioWorkletProcessor {
  private framer = new Framer(sampleRate, (f) => this.port.postMessage(f, [f.pcm]))

  /** Input 0 is the microphone, input 1 the tab (empty in room mode); each downmixed to one channel. */
  process(inputs: Float32Array[][]) {
    this.framer.push(inputs.map((channels) => channels[0]))
    return true
  }
}

registerProcessor('recapz-pcm', PcmProcessor)
