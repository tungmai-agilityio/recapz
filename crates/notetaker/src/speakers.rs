//! Match transcript segments to the active-speaker samples read from the call page.

/// One sample of who was marked as speaking at `at_ms` (taken about every 250 ms).
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub at_ms: i64,
    pub speaker: String,
}

/// The speaker with the most samples inside `[start_ms, end_ms]`, or `None` when nobody was sampled.
/// Both ends are inclusive, so a sample on a shared boundary counts for both neighbours (intended).
/// Ties go to the speaker who appears first in the window.
pub fn speaker_for(samples: &[Sample], start_ms: i64, end_ms: i64) -> Option<&str> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for s in samples
        .iter()
        .filter(|s| s.at_ms >= start_ms && s.at_ms <= end_ms)
    {
        match counts.iter_mut().find(|(n, _)| *n == s.speaker) {
            Some((_, c)) => *c += 1,
            None => counts.push((&s.speaker, 1)),
        }
    }
    // max_by_key keeps the last maximum, so reverse to prefer the earliest speaker on ties.
    counts
        .into_iter()
        .rev()
        .max_by_key(|(_, c)| *c)
        .map(|(n, _)| n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(at_ms: i64, speaker: &str) -> Sample {
        Sample {
            at_ms,
            speaker: speaker.into(),
        }
    }

    #[test]
    fn picks_the_majority_speaker_in_the_window() {
        let v = [
            s(0, "A"),
            s(250, "B"),
            s(500, "B"),
            s(750, "B"),
            s(1000, "A"),
            s(5000, "A"),
        ];
        assert_eq!(speaker_for(&v, 0, 1000), Some("B"));
        assert_eq!(speaker_for(&v, 1000, 6000), Some("A"));
    }

    #[test]
    fn none_when_no_samples_in_window() {
        assert_eq!(speaker_for(&[s(0, "A")], 100, 200), None);
        assert_eq!(speaker_for(&[], 0, 10), None);
    }

    #[test]
    fn tie_goes_to_first_speaker() {
        let v = [s(0, "A"), s(250, "B")];
        assert_eq!(speaker_for(&v, 0, 500), Some("A"));
    }
}
