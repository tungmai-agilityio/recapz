use anyhow::Result;

/// One recognized span of speech. Times are milliseconds from the start of the audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
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
            },
            Segment {
                start_ms: half,
                end_ms: total_ms,
                text: "Let's start the weekly sync.".into(),
            },
        ])
    }
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

    impl Transcriber for WhisperTranscriber {
        fn transcribe(&self, samples: &[f32], language: &str) -> Result<Vec<Segment>> {
            let mut state = self.0.create_state()?;
            let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            // "vi+en" meetings are mixed: let the model detect the language per call.
            p.set_language(if language == "vi" || language == "en" {
                Some(language)
            } else {
                None
            });
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
                    })
                })
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
}
