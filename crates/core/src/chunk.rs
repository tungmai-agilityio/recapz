//! Splits a transcript into search chunks of about one minute that overlap by ten seconds,
//! so a sentence cut at one boundary still appears whole in the next chunk.

pub const WINDOW_MS: i64 = 60_000;
pub const OVERLAP_MS: i64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// `segments` are `(start_ms, end_ms, text)` in time order. Segments are never split.
pub fn chunk(segments: &[(i64, i64, String)]) -> Vec<Chunk> {
    let mut out = vec![];
    let mut i = 0;
    while i < segments.len() {
        let start = segments[i].0;
        let mut j = i;
        while j + 1 < segments.len() && segments[j].1 - start < WINDOW_MS {
            j += 1;
        }
        let end = segments[j].1;
        let text: Vec<&str> = segments[i..=j].iter().map(|s| s.2.as_str()).collect();
        out.push(Chunk {
            start_ms: start,
            end_ms: end,
            text: text.join(" "),
        });
        if j + 1 >= segments.len() {
            break;
        }
        // Next chunk starts at the first segment inside the overlap, but always moves forward.
        let next = (i + 1..=j)
            .find(|&k| segments[k].0 >= end - OVERLAP_MS)
            .unwrap_or(j + 1);
        i = next.max(i + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_s: i64, end_s: i64, t: &str) -> (i64, i64, String) {
        (start_s * 1000, end_s * 1000, t.to_string())
    }

    #[test]
    fn empty_transcript_has_no_chunks() {
        assert!(chunk(&[]).is_empty());
    }

    #[test]
    fn short_transcript_is_one_chunk() {
        let c = chunk(&[seg(0, 5, "a"), seg(5, 10, "b")]);
        assert_eq!(
            c,
            vec![Chunk {
                start_ms: 0,
                end_ms: 10_000,
                text: "a b".into()
            }]
        );
    }

    #[test]
    fn long_transcript_overlaps_by_ten_seconds() {
        // Twelve 10-second segments: 0..120 s.
        let segs: Vec<_> = (0..12)
            .map(|i| seg(i * 10, i * 10 + 10, &format!("s{i}")))
            .collect();
        let c = chunk(&segs);
        // 1st chunk covers s0..s5 (0-60 s); the 2nd starts at s5 (50 s, inside the overlap).
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].start_ms, c[0].end_ms), (0, 60_000));
        assert_eq!(c[0].text, "s0 s1 s2 s3 s4 s5");
        assert_eq!((c[1].start_ms, c[1].end_ms), (50_000, 110_000));
        assert_eq!(c[1].text, "s5 s6 s7 s8 s9 s10");
        assert_eq!((c[2].start_ms, c[2].end_ms), (100_000, 120_000));
        assert_eq!(c[2].text, "s10 s11");
    }

    #[test]
    fn one_huge_segment_still_progresses() {
        let c = chunk(&[seg(0, 300, "long"), seg(300, 301, "tail")]);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].text, "long");
        assert_eq!(c[1].text, "tail");
    }
}
