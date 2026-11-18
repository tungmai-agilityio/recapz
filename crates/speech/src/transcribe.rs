use anyhow::Result;

/// One recognized span of speech. Times are milliseconds from the start of the audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    /// Provider's speaker tag, when it gives one. Only equality matters; the job names the rows.
    pub speaker: Option<String>,
}

/// Turns 16 kHz mono samples into timed segments. `language` is a hint such as "vi" or "auto".
pub trait Transcriber: Send + Sync {
    fn transcribe(&self, samples: &[f32], language: &str) -> Result<Vec<Segment>>;
}

/// Returns fixed segments scaled to the audio length. Used by tests and when no model is configured.
pub struct StubTranscriber;

impl Transcriber for StubTranscriber {
    fn transcribe(&self, samples: &[f32], _language: &str) -> Result<Vec<Segment>> {
        let total_ms = (samples.len() as i64) * 1000 / crate::decode::TARGET_RATE as i64;
        let half = total_ms / 2;
        Ok(vec![
            Segment {
                start_ms: 0,
                end_ms: half,
                text: "Xin chào mọi người.".into(),
                speaker: None,
            },
            Segment {
                start_ms: half,
                end_ms: total_ms,
                text: "Let's start the weekly sync.".into(),
                speaker: None,
            },
        ])
    }
}

/// Start (ms) of the 30 s window with the most energy, where language detection has speech to hear.
/// Detecting on the first window fails when a call opens with silence or music.
// ponytail: one language per recording; per-chunk detection if one meeting switches language for long stretches
pub fn loudest_window_ms(samples: &[f32]) -> usize {
    const WIN: usize = 30;
    let rate = crate::decode::TARGET_RATE as usize;
    let secs: Vec<f64> = samples
        .chunks(rate)
        .map(|c| c.iter().map(|x| (*x as f64).powi(2)).sum::<f64>())
        .collect();
    if secs.len() <= WIN {
        return 0;
    }
    let mut sum: f64 = secs[..WIN].iter().sum();
    let (mut best, mut at) = (sum, 0);
    for i in WIN..secs.len() {
        sum += secs[i] - secs[i - WIN];
        if sum > best {
            (best, at) = (sum, i - WIN + 1);
        }
    }
    at * 1000
}

/// True for text that is only a non-speech marker such as "[BLANK_AUDIO]" or "(soft music)".
pub fn is_non_speech(text: &str) -> bool {
    let t = text.trim();
    t.is_empty()
        || (t.starts_with('[') && t.ends_with(']'))
        || (t.starts_with('(') && t.ends_with(')'))
}

#[cfg(feature = "whisper")]
pub use real::WhisperTranscriber;

#[cfg(feature = "whisper")]
mod real {
    use super::*;
    use anyhow::Context;
    use std::path::Path;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    pub struct WhisperTranscriber(WhisperContext);

    impl WhisperTranscriber {
        pub fn load(model: &Path) -> Result<Self> {
            let path = model.to_str().context("model path is not UTF-8")?;
            Ok(Self(
                WhisperContext::new_with_params(path, WhisperContextParameters::default())
                    .context("load speech model")?,
            ))
        }
    }

    /// Picks "vi" or "en" from the language probabilities of the loudest 30 s of the recording.
    fn detect_vi_en(state: &mut whisper_rs::WhisperState, samples: &[f32]) -> Result<&'static str> {
        let id = |l| whisper_rs::get_lang_id(l).context("language id");
        let (vi, en) = (id("vi")? as usize, id("en")? as usize);
        let from = loudest_window_ms(samples) * crate::decode::TARGET_RATE as usize / 1000;
        let window =
            &samples[from..(from + 30 * crate::decode::TARGET_RATE as usize).min(samples.len())];
        state.pcm_to_mel(window, 4)?;
        let (_, probs) = state.lang_detect(0, 4)?;
        Ok(if probs[vi] > probs[en] { "vi" } else { "en" })
    }

    impl Transcriber for WhisperTranscriber {
        fn transcribe(&self, samples: &[f32], language: &str) -> Result<Vec<Segment>> {
            let mut state = self.0.create_state()?;
            let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            // "vi+en" meetings are mixed. Whisper's own auto-detect may pick any of ~100 languages
            // (a quiet or music-only start gave Norwegian), so choose between vi and en only.
            p.set_language(match language {
                "vi" => Some("vi"),
                "en" => Some("en"),
                // a failed detection falls back to Whisper's own, as before
                "vi+en" => detect_vi_en(&mut state, samples).ok(),
                _ => None,
            });
            // Do not feed each window's text to the next: one wrong line otherwise repeats for the whole call.
            p.set_no_context(true);
            p.set_print_progress(false);
            p.set_print_realtime(false);
            p.set_print_special(false);
            p.set_print_timestamps(false);
            state.full(p, samples)?;
            let n = state.full_n_segments()?;
            (0..n)
                .map(|i| {
                    Ok(Segment {
                        // whisper timestamps are in 10 ms units
                        start_ms: state.full_get_segment_t0(i)? * 10,
                        end_ms: state.full_get_segment_t1(i)? * 10,
                        text: state.full_get_segment_text_lossy(i)?.trim().to_string(),
                        speaker: None,
                    })
                })
                .filter(|s| !s.as_ref().is_ok_and(|s| is_non_speech(&s.text)))
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_segments_cover_the_audio_exactly() {
        let t: Box<dyn Transcriber> = Box::new(StubTranscriber);
        let segs = t.transcribe(&vec![0.0; 32_000], "vi+en").unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!((segs[0].start_ms, segs[0].end_ms), (0, 1000));
        assert_eq!((segs[1].start_ms, segs[1].end_ms), (1000, 2000));
        assert_eq!(segs[1].text, "Let's start the weekly sync.");
    }

    #[test]
    fn loudest_window_skips_leading_silence() {
        let rate = crate::decode::TARGET_RATE as usize;
        let mut samples = vec![0.0f32; 20 * rate];
        samples.extend(std::iter::repeat_n(0.5f32, 30 * rate));
        samples.extend(vec![0.0f32; 20 * rate]);
        assert_eq!(loudest_window_ms(&samples), 20_000);
        assert_eq!(loudest_window_ms(&vec![0.5f32; 10 * rate]), 0);
    }

    #[test]
    fn non_speech_markers_are_dropped_but_speech_with_brackets_is_kept() {
        assert!(is_non_speech("[BLANK_AUDIO]"));
        assert!(is_non_speech(" (soft music) "));
        assert!(is_non_speech(""));
        assert!(!is_non_speech("Launch on March 14th (maybe)"));
        assert!(!is_non_speech("[Sam] agreed"));
    }
}
