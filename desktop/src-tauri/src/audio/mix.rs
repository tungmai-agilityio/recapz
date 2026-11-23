//! Sums two 16 kHz mono streams (system audio and microphone) into one, frame by frame. Pure logic, so it is
//! unit tested. Each source delivers whole 1600-sample frames at its own pace; a frame goes out as soon as both
//! have one. A source that stalls (permission pending, device swap) never holds the other back for more than
//! one extra frame: past that, its missing half is silence.
use super::frame::FRAME_SAMPLES;
use std::collections::VecDeque;

#[derive(Default)]
pub struct Mixer {
    sides: [VecDeque<i16>; 2],
}

impl Mixer {
    /// Feeds one source (`side` 0 or 1) and calls `emit` for every frame that is ready.
    pub fn push(&mut self, side: usize, samples: &[i16], mut emit: impl FnMut(&[i16])) {
        self.sides[side].extend(samples);
        loop {
            let (a, b) = (self.sides[0].len(), self.sides[1].len());
            let both = a >= FRAME_SAMPLES && b >= FRAME_SAMPLES;
            let lagging = a >= 2 * FRAME_SAMPLES || b >= 2 * FRAME_SAMPLES;
            if !both && !lagging {
                return;
            }
            let mut frame = [0i16; FRAME_SAMPLES];
            for side in &mut self.sides {
                for (out, s) in frame
                    .iter_mut()
                    .zip(side.drain(..FRAME_SAMPLES.min(side.len())))
                {
                    *out = out.saturating_add(s);
                }
            }
            emit(&frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(steps: &[(usize, i16, usize)]) -> Vec<Vec<i16>> {
        let mut m = Mixer::default();
        let mut out = Vec::new();
        for &(side, value, n) in steps {
            m.push(side, &vec![value; n], |f| out.push(f.to_vec()));
        }
        out
    }

    #[test]
    fn sums_matching_frames_at_exact_size() {
        let out = run(&[(0, 100, 1600), (1, -30, 1600)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), FRAME_SAMPLES);
        assert!(out[0].iter().all(|&s| s == 70));
    }

    #[test]
    fn clamps_instead_of_wrapping() {
        let out = run(&[
            (0, 30_000, 1600),
            (1, 30_000, 1600),
            (0, -30_000, 1600),
            (1, -30_000, 1600),
        ]);
        assert_eq!(out.len(), 2);
        assert!(out[0].iter().all(|&s| s == i16::MAX));
        assert!(out[1].iter().all(|&s| s == i16::MIN));
    }

    #[test]
    fn waits_for_the_other_source_then_emits_whole_frames_only() {
        // 1599 + 1 samples on one side, nothing on the other: no frame yet.
        assert!(run(&[(0, 5, 1599), (0, 5, 1)]).is_empty());
        // Two frames on one side go out at once with silence for the other; the late frame then pairs up.
        let out = run(&[(0, 1, 3200), (1, 2, 1600)]);
        assert_eq!(out.len(), 2);
        assert!(out[0].iter().all(|&s| s == 1)); // the stalled side is silence
        assert!(out[1].iter().all(|&s| s == 3));
    }

    #[test]
    fn a_silent_source_adds_at_most_one_frame_of_delay() {
        let mut m = Mixer::default();
        let mut n = 0;
        m.push(0, &vec![7; 1600], |_| n += 1);
        assert_eq!(n, 0);
        m.push(0, &vec![7; 1599], |_| n += 1);
        assert_eq!(n, 0);
        m.push(0, &[7], |_| n += 1);
        assert_eq!(n, 1);
    }
}
