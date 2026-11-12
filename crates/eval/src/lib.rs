//! Word and character error rates for comparing speech models (see docs/speech-model-comparison.md).
use unicode_normalization::UnicodeNormalization;

pub mod ask;

/// NFC, lowercase, punctuation removed, whitespace collapsed.
/// ponytail: tone-mark placement (`hoà` vs `hòa`) is not unified; add if it shows up in real diffs.
pub fn normalize(s: &str) -> String {
    let cleaned: String = s
        .nfc()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Removes tone and vowel marks, so tone errors can be read apart from word errors.
pub fn strip_diacritics(s: &str) -> String {
    s.nfd()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .map(|c| if c == 'đ' || c == 'Đ' { 'd' } else { c })
        .collect()
}

/// Edit distance between two sequences (substitutions, insertions and deletions cost 1).
fn distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, y) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(x != y))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

fn rate(errors: usize, reference_len: usize) -> f64 {
    if reference_len == 0 {
        return if errors == 0 { 0.0 } else { 1.0 };
    }
    errors as f64 / reference_len as f64
}

/// Word error rate of normalized text. Vietnamese words are syllables, so this is a syllable error rate.
pub fn wer(reference: &str, hypothesis: &str) -> f64 {
    let (r, h) = (normalize(reference), normalize(hypothesis));
    let (r, h): (Vec<_>, Vec<_>) = (
        r.split(' ').filter(|w| !w.is_empty()).collect(),
        h.split(' ').filter(|w| !w.is_empty()).collect(),
    );
    rate(distance(&r, &h), r.len())
}

/// Character error rate of normalized text (spaces count).
pub fn cer(reference: &str, hypothesis: &str) -> f64 {
    let (r, h): (Vec<char>, Vec<char>) = (
        normalize(reference).chars().collect(),
        normalize(hypothesis).chars().collect(),
    );
    rate(distance(&r, &h), r.len())
}

pub struct Report {
    pub wer: f64,
    pub cer: f64,
    pub wer_no_diacritics: f64,
}

pub fn report(reference: &str, hypothesis: &str) -> Report {
    Report {
        wer: wer(reference, hypothesis),
        cer: cer(reference, hypothesis),
        wer_no_diacritics: wer(&strip_diacritics(reference), &strip_diacritics(hypothesis)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_substitutions_and_deletions() {
        // "b"→"x" is one substitution, "d" is one deletion: 2 errors over 4 reference words.
        assert_eq!(wer("a b c d", "a x c"), 0.5);
        assert_eq!(wer("a b", "a b"), 0.0);
        assert_eq!(wer("a b", "a b c d"), 1.0); // two insertions over two words
    }

    #[test]
    fn character_errors_are_counted_per_character() {
        assert_eq!(cer("abcd", "abxd"), 0.25);
    }

    #[test]
    fn normalization_ignores_case_and_punctuation() {
        assert_eq!(wer("Xin chào, mọi người!", "xin chào mọi người"), 0.0);
    }

    #[test]
    fn tone_errors_show_in_wer_but_not_in_the_no_diacritics_rate() {
        let r = report("tôi đi học", "toi di hoc");
        assert_eq!(r.wer, 1.0);
        assert_eq!(r.wer_no_diacritics, 0.0);
    }

    #[test]
    fn capital_d_with_stroke_is_stripped_too() {
        assert_eq!(report("Đi học", "di hoc").wer_no_diacritics, 0.0);
        assert_eq!(strip_diacritics("Đường"), "duong");
    }

    #[test]
    fn empty_reference_is_defined() {
        assert_eq!(wer("", ""), 0.0);
        assert_eq!(wer("", "extra"), 1.0);
    }
}
