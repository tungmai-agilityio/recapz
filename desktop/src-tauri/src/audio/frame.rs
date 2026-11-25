//! Turns captured float audio into the frames the page sends to the socket: 16 kHz mono i16, 1600 samples
//! (100 ms) each. Pure logic, no platform calls, so it is unit tested.

pub const RATE: u32 = 16_000;
pub const FRAME_SAMPLES: usize = 1600;

/// Averages interleaved or planar buffers down to one channel. Each buffer is `(samples, channels in it)`; the
/// tap gives one interleaved buffer, other devices one buffer per channel. Frames are cut to the shortest buffer.
pub fn downmix(buffers: &[(&[f32], usize)]) -> Vec<f32> {
    let total: usize = buffers.iter().map(|(_, c)| c).sum();
    let frames = buffers
        .iter()
        .map(|(d, c)| d.len() / c.max(&1))
        .min()
        .unwrap_or(0);
    (0..frames)
        .map(|i| {
            let sum: f32 = buffers
                .iter()
                .map(|(d, c)| d[i * c..(i + 1) * c].iter().sum::<f32>())
                .sum();
            sum / total as f32
        })
        .collect()
}

/// Cutoff of the low-pass as a share of the output rate (7.2 kHz at 16 kHz), and its half-width in output samples.
/// A Blackman-windowed sinc this long is flat to about 6 kHz and down more than 60 dB from 9 kHz, so 8 to 16 kHz
/// content cannot fold back into the speech band.
const CUTOFF: f64 = 0.45;
const HALF_OUT: f64 = 24.0;
/// Fractional positions kept in the filter table; rates whose reduced ratio needs fewer are exact.
const PHASES: u64 = 256;

/// Streaming polyphase windowed-sinc resampler. Output k sits at input position `k * from / to`, tracked in
/// integers so a whole number of seconds in gives exactly that many seconds out, at any rate. It runs
/// `half` input samples behind (1.5 ms at 48 kHz), and the start is padded with the first sample so a steady
/// signal keeps its level.
pub struct Resampler {
    f: u64,
    t: u64,
    half: usize,
    ph: u64,
    c: usize,
    primed: bool,
    pass: bool,
    phases: Vec<Vec<f32>>,
    buf: Vec<f32>,
}

impl Resampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        let (from, to) = (from_rate as u64, to_rate as u64);
        // A device mid-switch can report rate 0: no taps and no output, so push returns at once.
        let pass = from == to;
        let (f, t, phases, half) = if from == 0 || to == 0 || pass {
            (1, 1, Vec::new(), 0)
        } else {
            let g = gcd(from, to);
            let (f, t) = (from / g, to / g);
            let ratio = from as f64 / to as f64;
            let half = (HALF_OUT * ratio.max(1.0)).ceil() as usize;
            let wc = CUTOFF / ratio.max(1.0);
            let count = t.min(PHASES);
            let phases = (0..count)
                .map(|p| kernel(p as f64 / count as f64, wc, half))
                .collect();
            (f, t, phases, half)
        };
        Self {
            f,
            t,
            half,
            ph: 0,
            c: half,
            primed: false,
            pass,
            phases,
            buf: Vec::new(),
        }
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.pass {
            out.extend_from_slice(input);
            return;
        }
        if self.phases.is_empty() || input.is_empty() {
            return;
        }
        if !self.primed {
            self.buf.resize(self.half, input[0]);
            self.primed = true;
        }
        self.buf.extend_from_slice(input);
        let count = self.phases.len() as u64;
        while self.c + self.half < self.buf.len() {
            let taps = &self.phases[(self.ph * count / self.t) as usize];
            let start = self.c + 1 - self.half;
            out.push(
                taps.iter()
                    .zip(&self.buf[start..start + 2 * self.half])
                    .map(|(a, b)| a * b)
                    .sum(),
            );
            self.ph += self.f;
            self.c += (self.ph / self.t) as usize;
            self.ph %= self.t;
        }
        let keep_from = self.c + 1 - self.half;
        self.buf.drain(..keep_from);
        self.c -= keep_from;
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Taps for an output sitting `frac` input samples past an input sample: input offsets `-half+1 ..= half`,
/// summing to exactly 1 so a constant keeps its level.
fn kernel(frac: f64, wc: f64, half: usize) -> Vec<f32> {
    use std::f64::consts::PI;
    let h = half as f64;
    let taps: Vec<f64> = (1..=2 * half as i64)
        .map(|j| {
            let u = (j - half as i64) as f64 - frac;
            let x = 2.0 * wc * u;
            let sinc = if x == 0.0 {
                1.0
            } else {
                (PI * x).sin() / (PI * x)
            };
            sinc * (0.42 + 0.5 * (PI * u / h).cos() + 0.08 * (2.0 * PI * u / h).cos())
        })
        .collect();
    let sum: f64 = taps.iter().sum();
    taps.iter().map(|t| (t / sum) as f32).collect()
}

/// Resampler plus i16 conversion plus cutting into whole frames; the remainder waits for the next push.
pub struct Framer {
    rate: u32,
    resampler: Resampler,
    pending: Vec<i16>,
}

impl Framer {
    pub fn new(from_rate: u32) -> Self {
        Self {
            rate: from_rate,
            resampler: Resampler::new(from_rate, RATE),
            pending: Vec::new(),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Feeds mono samples; calls `emit` once per complete frame.
    pub fn push(&mut self, mono: &[f32], mut emit: impl FnMut(&[i16])) {
        let mut out = Vec::with_capacity(mono.len() / 2 + 1);
        self.resampler.push(mono, &mut out);
        self.pending.extend(
            out.iter()
                .map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16),
        );
        let whole = self.pending.len() / FRAME_SAMPLES * FRAME_SAMPLES;
        for f in self.pending[..whole].chunks(FRAME_SAMPLES) {
            emit(f);
        }
        self.pending.drain(..whole);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, hz: f64, seconds: usize) -> Vec<f32> {
        (0..rate as usize * seconds)
            .map(|n| {
                (0.5 * (2.0 * std::f64::consts::PI * hz * n as f64 / rate as f64).sin()) as f32
            })
            .collect()
    }

    /// RMS of the resampled tone, skipping the first and last 100 outputs (filter start and end).
    fn rms_after(rate: u32, hz: f64) -> f64 {
        let mut out = Vec::new();
        Resampler::new(rate, RATE).push(&tone(rate, hz, 1), &mut out);
        let mid = &out[100..out.len() - 100];
        (mid.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / mid.len() as f64).sqrt()
    }

    fn frames_for(rate: u32, samples: usize, chunk: usize) -> usize {
        let mut f = Framer::new(rate);
        let mut n = 0;
        let input = vec![0.5f32; samples];
        for c in input.chunks(chunk) {
            f.push(c, |fr| {
                assert_eq!(fr.len(), FRAME_SAMPLES);
                n += 1;
            });
        }
        n
    }

    #[test]
    fn a_bit_over_one_second_is_ten_frames_at_common_rates() {
        // The filter runs about 1.5 ms behind, so the tenth frame needs 10 ms more input than a second.
        assert_eq!(frames_for(48_000, 48_480, 512), 10);
        assert_eq!(frames_for(44_100, 44_541, 441), 10);
        assert_eq!(frames_for(16_000, 16_000, 1000), 10);
        assert_eq!(frames_for(48_000, 48_000, 512), 9);
    }

    #[test]
    fn speech_band_tones_pass_and_content_above_the_band_is_removed() {
        // A tone of amplitude 0.5 has RMS 0.3536. Above 8 kHz the old box filter folded these back in
        // (a 10 kHz tone came out at half amplitude as a 6 kHz tone).
        for rate in [48_000, 44_100] {
            for hz in [300.0, 1000.0, 4000.0, 6000.0] {
                let rms = rms_after(rate, hz);
                assert!((rms - 0.3536).abs() < 0.007, "{rate} {hz}: {rms}");
            }
            for hz in [10_000.0, 12_000.0, 20_000.0] {
                let rms = rms_after(rate, hz);
                assert!(rms < 0.0035, "{rate} {hz}: {rms}");
            }
        }
    }

    #[test]
    fn chunking_does_not_change_the_output() {
        let input = tone(44_100, 1000.0, 1);
        let (mut whole, mut parts) = (Vec::new(), Vec::new());
        Resampler::new(44_100, RATE).push(&input, &mut whole);
        let mut r = Resampler::new(44_100, RATE);
        for c in input.chunks(97) {
            r.push(c, &mut parts);
        }
        assert_eq!(whole, parts);
    }

    #[test]
    fn exact_sample_counts_and_remainder_carries_over() {
        let mut r = Resampler::new(44_100, RATE);
        let mut out = Vec::new();
        r.push(&vec![0.0; 44_100], &mut out);
        // The filter runs 67 input samples behind: 24 outputs short of a second.
        assert_eq!(out.len(), 15_976);
        // 100 more input samples are not yet a whole number of outputs: 100 * 16000 / 44100 = 36.28.
        out.clear();
        r.push(&vec![0.0; 100], &mut out);
        assert_eq!(out.len(), 36);
    }

    #[test]
    fn rate_zero_yields_nothing_and_does_not_hang() {
        let mut f = Framer::new(0);
        f.push(&[0.5; 100], |_| panic!("no frame expected"));
    }

    #[test]
    fn partial_frame_is_held_back() {
        let mut f = Framer::new(48_000);
        let mut n = 0;
        f.push(&vec![0.0; 4869], |_| n += 1); // 1599 output samples (the filter runs 72 input samples behind)
        assert_eq!(n, 0);
        f.push(&[0.0; 1], |_| n += 1); // 1600
        assert_eq!(n, 1);
    }

    #[test]
    fn constant_signal_keeps_level_and_clamps() {
        let mut f = Framer::new(48_000);
        let mut got = Vec::new();
        f.push(&vec![0.5; 4900], |fr| got.extend_from_slice(fr));
        assert_eq!(got.len(), 1600);
        assert!(got.iter().all(|&s| s == 16384)); // 0.5 * 32767 = 16383.5, rounds to 16384
        let mut loud = Vec::new();
        Framer::new(16_000).push(&vec![2.0; 1600], |fr| loud.extend_from_slice(fr));
        assert!(loud.iter().all(|&s| s == 32767));
    }

    #[test]
    fn downmix_interleaved_and_planar() {
        let st = [1.0, 3.0, 2.0, 4.0]; // two stereo frames
        assert_eq!(downmix(&[(&st, 2)]), vec![2.0, 3.0]);
        let (l, r) = ([1.0, 2.0], [3.0, 4.0]);
        assert_eq!(downmix(&[(&l, 1), (&r, 1)]), vec![2.0, 3.0]);
        assert!(downmix(&[]).is_empty());
    }
}
