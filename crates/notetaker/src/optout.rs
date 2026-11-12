//! Detect an opt-out request in call chat.

// Compared as whole words after splitting on punctuation, so "opt-out" is covered by "opt out".
// Chat text is expected in NFC form; decomposed diacritics are not matched.
const PHRASES: [&str; 5] = [
    "opt out",
    "optout",
    "từ chối ghi âm",
    "tu choi ghi am",
    "tắt ghi âm",
];

/// True when a chat message asks the notetaker to stop recording.
/// Case-insensitive; matches whole words only, so "adopt outcomes" is not an opt-out.
pub fn is_opt_out(message: &str) -> bool {
    let words: Vec<String> = message
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let padded = format!(" {} ", words.join(" "));
    PHRASES.iter().any(|p| padded.contains(&format!(" {p} ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_english_and_vietnamese() {
        for m in [
            "opt out",
            "Please OPT   OUT now",
            "I opt-out",
            "optout",
            "Từ chối ghi âm",
            "tu choi ghi am",
            "Tắt ghi âm đi",
        ] {
            assert!(is_opt_out(m), "{m}");
        }
    }

    #[test]
    fn ignores_ordinary_chat() {
        for m in [
            "",
            "let's opt for plan B",
            "we ran out of time",
            "ghi âm ổn rồi",
            "I'll opt in",
            "let's adopt outcomes",
            "adopt out",
            "the optouts",
        ] {
            assert!(!is_opt_out(m), "{m}");
        }
    }
}
