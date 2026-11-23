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

/// Streaming box-filter resampler. It works in integer units (an input sample is `to` wide, an output sample
/// `from` wide), so a whole number of seconds in gives exactly that many seconds out, at any rate.
pub struct Resampler {
    from: u64,
    to: u64,
    filled: u64,
    sum: f64,
}

impl Resampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        Self {
            from: from_rate as u64,
            to: to_rate as u64,
            filled: 0,
            sum: 0.0,
        }
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for &x in input {
            let mut left = self.to;
            while left > 0 {
                let take = left.min(self.from - self.filled);
                self.sum += x as f64 * take as f64;
                self.filled += take;
                left -= take;
                if self.filled == self.from {
                    out.push((self.sum / self.from as f64) as f32);
                    self.sum = 0.0;
                    self.filled = 0;
                }
            }
        }
    }
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
    fn one_second_is_ten_frames_at_common_rates() {
        assert_eq!(frames_for(48_000, 48_000, 512), 10);
        assert_eq!(frames_for(44_100, 44_100, 441), 10);
        assert_eq!(frames_for(16_000, 16_000, 1000), 10);
    }

    #[test]
    fn exact_sample_counts_and_remainder_carries_over() {
        let mut r = Resampler::new(44_100, RATE);
        let mut out = Vec::new();
        r.push(&vec![0.0; 44_100], &mut out);
        assert_eq!(out.len(), 16_000);
        // 100 more input samples are not yet a whole number of outputs: 100 * 16000 / 44100 = 36.28.
        out.clear();
        r.push(&vec![0.0; 100], &mut out);
        assert_eq!(out.len(), 36);
    }

    #[test]
    fn partial_frame_is_held_back() {
        let mut f = Framer::new(48_000);
        let mut n = 0;
        f.push(&vec![0.0; 4799], |_| n += 1); // 1599 output samples
        assert_eq!(n, 0);
        f.push(&[0.0; 3], |_| n += 1); // 1600
        assert_eq!(n, 1);
    }

    #[test]
    fn constant_signal_keeps_level_and_clamps() {
        let mut f = Framer::new(48_000);
        let mut got = Vec::new();
        f.push(&vec![0.5; 4800], |fr| got.extend_from_slice(fr));
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
