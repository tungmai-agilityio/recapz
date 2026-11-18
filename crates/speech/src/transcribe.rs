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

/// True for text that is only a non-speech marker such as "[BLANK_AUDIO]" or "(soft music)".
pub fn is_non_speech(text: &str) -> bool {
    let t = text.trim();
    t.is_empty()
        || (t.starts_with('[') && t.ends_with(']'))
        || (t.starts_with('(') && t.ends_with(')'))
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
    fn non_speech_markers_are_dropped_but_speech_with_brackets_is_kept() {
        assert!(is_non_speech("[BLANK_AUDIO]"));
        assert!(is_non_speech(" (soft music) "));
        assert!(is_non_speech(""));
        assert!(!is_non_speech("Launch on March 14th (maybe)"));
        assert!(!is_non_speech("[Sam] agreed"));
    }
}
