//! Recognise a pasted or calendar meeting link and normalise it. Only https links on the real
//! Meet and Zoom hosts are accepted, so a look-alike host never reaches the browser.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// Normalised to `https://meet.google.com/<code>`.
    Meet { code: String, url: String },
    /// `passcode` is the `pwd` query value when present.
    Zoom {
        number: String,
        passcode: Option<String>,
        url: String,
    },
}

impl Link {
    pub fn platform(&self) -> &'static str {
        match self {
            Link::Meet { .. } => "meet",
            Link::Zoom { .. } => "zoom",
        }
    }

    pub fn url(&self) -> &str {
        match self {
            Link::Meet { url, .. } | Link::Zoom { url, .. } => url,
        }
    }
}

fn strip_https(s: &str) -> Option<&str> {
    (s.len() > 8 && s.is_char_boundary(8) && s[..8].eq_ignore_ascii_case("https://"))
        .then(|| &s[8..])
}

/// `abc-defg-hij`: three lowercase letters, four, three.
fn is_meet_code(s: &str) -> bool {
    let p: Vec<&str> = s.split('-').collect();
    p.len() == 3
        && [3, 4, 3]
            .iter()
            .zip(&p)
            .all(|(n, x)| x.len() == *n && x.bytes().all(|b| b.is_ascii_lowercase()))
}

/// `zoom.us` or a subdomain with plain labels. Other Zoom domains (government, regional) are not
/// supported on purpose.
fn is_zoom_host(h: &str) -> bool {
    let plain = |l: &str| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-');
    match h.strip_suffix(".zoom.us") {
        Some(sub) => sub.split('.').all(plain),
        None => h == "zoom.us",
    }
}

/// Returns `None` for anything that is not a joinable Meet or Zoom link.
pub fn parse(raw: &str) -> Option<Link> {
    let raw = raw.trim().split('#').next().unwrap_or(""); // drop the fragment first
    let rest = strip_https(raw)?;
    let (host_path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = host_path.split_once('/').unwrap_or((host_path, ""));
    // No userinfo or port tricks: the host is only letters, digits, dots and hyphens.
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return None;
    }
    let host = host.to_ascii_lowercase();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if host == "meet.google.com" {
        return match segs.as_slice() {
            [code] if is_meet_code(code) => Some(Link::Meet {
                code: code.to_string(),
                url: format!("https://meet.google.com/{code}"),
            }),
            _ => None,
        };
    }
    if is_zoom_host(&host) {
        let number = match segs.as_slice() {
            ["j", n] | ["wc", "join", n] | ["wc", n, "join"] => *n,
            _ => return None,
        };
        if !(9..=11).contains(&number.len()) || !number.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let passcode = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("pwd="))
            .filter(|p| {
                !p.is_empty()
                    && p.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
            })
            .map(str::to_string);
        let url = match &passcode {
            Some(p) => format!("https://{host}/j/{number}?pwd={p}"),
            None => format!("https://{host}/j/{number}"),
        };
        return Some(Link::Zoom {
            number: number.to_string(),
            passcode,
            url,
        });
    }
    None
}

/// First joinable link found in free text such as an event location or description.
pub fn find_in_text(text: &str) -> Option<Link> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '(' | ')'))
        .find_map(|w| parse(w.trim_end_matches(['.', ',', ';', '!', ']', '}'])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meet_links_are_normalised() {
        let l = parse(" https://meet.google.com/abc-defg-hij?authuser=1 ").unwrap();
        assert_eq!(
            l,
            Link::Meet {
                code: "abc-defg-hij".into(),
                url: "https://meet.google.com/abc-defg-hij".into()
            }
        );
        assert_eq!(
            (l.platform(), l.url()),
            ("meet", "https://meet.google.com/abc-defg-hij")
        );
    }

    #[test]
    fn zoom_links_keep_number_and_passcode() {
        let l = parse("https://us02web.zoom.us/j/84012345678?pwd=Ab-c_1.2&uname=x").unwrap();
        assert_eq!(
            l,
            Link::Zoom {
                number: "84012345678".into(),
                passcode: Some("Ab-c_1.2".into()),
                url: "https://us02web.zoom.us/j/84012345678?pwd=Ab-c_1.2".into()
            }
        );
        let l = parse("https://zoom.us/wc/join/123456789").unwrap();
        assert_eq!(l.url(), "https://zoom.us/j/123456789");
        assert_eq!(l.platform(), "zoom");
    }

    #[test]
    fn rejects_other_hosts_schemes_and_malformed_ids() {
        for bad in [
            "",
            "meet.google.com/abc-defg-hij",
            "http://meet.google.com/abc-defg-hij",
            "https://meet.google.com.evil.com/abc-defg-hij",
            "https://evil.com/meet.google.com/abc-defg-hij",
            "https://user@meet.google.com/abc-defg-hij",
            "https://meet.google.com:8443/abc-defg-hij",
            "https://meet.google.com/abc-defg",
            "https://meet.google.com/ABC-DEFG-HIJ",
            "https://meet.google.com/abc-defg-hij/extra",
            "https://zoom.us.evil.com/j/123456789",
            "https://notzoom.us/j/123456789",
            "https://zoom.us/j/12345",
            "https://zoom.us/j/12345678x9",
            "https://zoom.us/my/someone",
            "https://meet.google.com\\@evil.com/abc-defg-hij",
            "https://meet.google.com@evil.com/abc-defg-hij",
            "https://evil.com#@meet.google.com/abc-defg-hij",
            "https://meet%2Egoogle.com/abc-defg-hij",
            "https://meet.googlé.com/abc-defg-hij",
            "https://zoom.us./j/123456789",
            "https://.zoom.us/j/123456789",
            "https://a..zoom.us/j/123456789",
            "https://-.zoom.us/j/123456789",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn fragments_do_not_hide_or_fake_the_passcode() {
        let l = parse("https://zoom.us/j/123456789?pwd=abc#success").unwrap();
        assert_eq!(
            l,
            Link::Zoom {
                number: "123456789".into(),
                passcode: Some("abc".into()),
                url: "https://zoom.us/j/123456789?pwd=abc".into()
            }
        );
        let l = parse("https://zoom.us/j/123456789#x?pwd=abc").unwrap();
        assert_eq!(
            l,
            Link::Zoom {
                number: "123456789".into(),
                passcode: None,
                url: "https://zoom.us/j/123456789".into()
            }
        );
    }

    #[test]
    fn host_case_scheme_case_and_the_other_zoom_shape_are_accepted() {
        assert_eq!(
            parse("HTTPS://MEET.GOOGLE.COM/abc-defg-hij").unwrap().url(),
            "https://meet.google.com/abc-defg-hij"
        );
        assert_eq!(
            parse("https://Us06web.Zoom.us/wc/123456789/join")
                .unwrap()
                .url(),
            "https://us06web.zoom.us/j/123456789"
        );
    }

    #[test]
    fn a_bad_passcode_is_dropped_not_passed_on() {
        let l = parse("https://zoom.us/j/123456789?pwd=a%20b").unwrap();
        assert_eq!(
            l,
            Link::Zoom {
                number: "123456789".into(),
                passcode: None,
                url: "https://zoom.us/j/123456789".into()
            }
        );
    }

    #[test]
    fn finds_the_first_joinable_link_in_text() {
        let t =
            "Agenda at https://example.com/doc, join: https://meet.google.com/abc-defg-hij. Thanks";
        assert_eq!(
            find_in_text(t).unwrap().url(),
            "https://meet.google.com/abc-defg-hij"
        );
        assert_eq!(find_in_text("no links here"), None);
        let t = "https://meet.google.com.evil.com/abc-defg-hij then https://zoom.us/j/123456789]!";
        assert_eq!(
            find_in_text(t).unwrap().url(),
            "https://zoom.us/j/123456789",
            "look-alike skipped, trailing punctuation trimmed"
        );
        assert_eq!(
            find_in_text("(https://zoom.us/j/123456789)")
                .unwrap()
                .platform(),
            "zoom"
        );
    }
}
