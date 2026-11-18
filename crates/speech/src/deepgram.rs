//! Deepgram batch transcriber: one blocking POST of the whole recording as 16-bit WAV, with speaker
//! labels. Errors name the HTTP status or failure kind only: never the key or any transcript text.
use crate::transcribe::{is_non_speech, Segment, Transcriber};
use anyhow::{anyhow, bail, Result};
use std::time::Duration;

/// The `language` query value for a meeting's language: `vi`, `en`, or `multi` (Nova-3 multilingual,
/// for mixed calls). `vi+en` and `auto`, which older meetings carry, mean `multi`. Anything else is
/// refused rather than silently sent as `multi`.
pub fn deepgram_language(language: &str) -> Option<&'static str> {
    match language {
        "vi" => Some("vi"),
        "en" => Some("en"),
        "multi" | "vi+en" | "auto" => Some("multi"),
        _ => None,
    }
}

pub const DEEPGRAM_BASE: &str = "https://api.deepgram.com";
const RETRIES: u32 = 2;
/// Longest wait honoured from `Retry-After`; a longer ask fails the job so the queue's own backoff takes over.
const MAX_WAIT: Duration = Duration::from_secs(30);

pub struct DeepgramTranscriber {
    base: String,
    key: String,
    http: reqwest::blocking::Client,
    /// Wait between retries when the provider gives no `Retry-After`.
    backoff: Duration,
}

impl DeepgramTranscriber {
    pub fn new(base_url: &str, api_key: &str) -> Result<Self> {
        Ok(Self {
            base: base_url.trim_end_matches('/').to_string(),
            key: api_key.to_string(),
            http: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(600))
                .build()
                .map_err(|e| anyhow!("http client: {}", e.without_url()))?,
            backoff: Duration::from_secs(2),
        })
    }
}

/// 16 kHz mono 16-bit PCM WAV, the format the notetaker records.
pub fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend(b"RIFF");
    w.extend((36 + data_len).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes()); // PCM
    w.extend(1u16.to_le_bytes()); // mono
    w.extend(crate::decode::TARGET_RATE.to_le_bytes());
    w.extend((crate::decode::TARGET_RATE * 2).to_le_bytes());
    w.extend(2u16.to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend(data_len.to_le_bytes());
    for s in samples {
        w.extend(((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    w
}

/// `results.utterances` as segments; the provider's speaker number is the tag.
fn parse(body: &str) -> Result<Vec<Segment>> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|_| anyhow!("unreadable speech response"))?;
    let utts = v["results"]["utterances"]
        .as_array()
        .ok_or_else(|| anyhow!("speech response has no utterances"))?;
    Ok(utts
        .iter()
        .filter_map(|u| {
            let text = u["transcript"].as_str()?.trim().to_string();
            Some(Segment {
                start_ms: (u["start"].as_f64()? * 1000.0).round() as i64,
                end_ms: (u["end"].as_f64()? * 1000.0).round() as i64,
                speaker: u["speaker"].as_i64().map(|n| n.to_string()),
                text,
            })
        })
        .filter(|s| !is_non_speech(&s.text))
        .collect())
}

impl Transcriber for DeepgramTranscriber {
    fn transcribe(&self, samples: &[f32], language: &str) -> Result<Vec<Segment>> {
        let lang =
            deepgram_language(language).ok_or_else(|| anyhow!("unsupported meeting language"))?;
        let url = format!(
            "{}/v1/listen?model=nova-3&language={lang}&diarize_model=latest&mip_opt_out=true&utterances=true&punctuate=true",
            self.base
        );
        let wav = encode_wav(samples);
        for attempt in 0..=RETRIES {
            let sent = self
                .http
                .post(&url)
                .header("Authorization", format!("Token {}", self.key))
                .header("Content-Type", "audio/wav")
                .body(wav.clone())
                .send();
            let wait = match sent {
                Ok(r) if r.status().is_success() => {
                    let body = r
                        .text()
                        .map_err(|e| anyhow!("speech response: {}", e.without_url()))?;
                    return parse(&body);
                }
                Ok(r) => {
                    let st = r.status();
                    if st.as_u16() != 429 && !st.is_server_error() {
                        // 400, 401, 402, 403 ...: retrying cannot help
                        bail!("speech provider refused the request: HTTP {st}");
                    }
                    if attempt == RETRIES {
                        bail!("speech provider unavailable: HTTP {st}");
                    }
                    r.headers()
                        .get("retry-after")
                        .and_then(|h| h.to_str().ok())
                        .and_then(|h| h.trim().parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or(self.backoff * (attempt + 1))
                }
                // timeouts and connection errors are worth another try; the error text is dropped
                Err(e) if attempt < RETRIES && (e.is_timeout() || e.is_connect()) => {
                    self.backoff * (attempt + 1)
                }
                Err(e) => bail!("speech provider unreachable ({})", kind(&e)),
            };
            if wait > MAX_WAIT {
                bail!("speech provider asked to wait {}s", wait.as_secs());
            }
            std::thread::sleep(wait);
        }
        unreachable!("the last attempt returns above")
    }
}

fn kind(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else {
        "request"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// Serves one canned (status, extra header, body) per request; sends each raw request back.
    fn mock(replies: Vec<(u16, &'static str, String)>) -> (String, mpsc::Receiver<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for (status, header, body) in replies {
                let (mut s, _) = l.accept().unwrap();
                let mut raw = Vec::new();
                let mut buf = [0u8; 8192];
                // read until headers end and the whole body arrived
                loop {
                    let n = s.read(&mut buf).unwrap();
                    raw.extend(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    if let Some(h) = text.find("\r\n\r\n") {
                        let len = text[..h]
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(|v| v.parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if raw.len() >= h + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                tx.send(String::from_utf8_lossy(&raw).to_string()).unwrap();
                write!(s, "HTTP/1.1 {status} X\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (url, rx)
    }

    const OK: &str = r#"{"results":{"utterances":[
        {"start":0.0,"end":1.52,"transcript":"Xin chào.","speaker":1},
        {"start":1.52,"end":3.0,"transcript":" Hello there ","speaker":0},
        {"start":3.0,"end":3.5,"transcript":"[music]","speaker":0},
        {"start":3.5,"end":4.0,"transcript":"Dạ.","speaker":1}]}}"#;

    fn client(url: &str) -> DeepgramTranscriber {
        let mut t = DeepgramTranscriber::new(url, "sekret-key").unwrap();
        t.backoff = Duration::from_millis(1);
        t
    }

    #[test]
    fn parses_utterances_with_speakers_and_sends_wav_with_the_key() {
        let (url, reqs) = mock(vec![(200, "", OK.into())]);
        let segs = client(&url).transcribe(&[0.25; 1600], "vi+en").unwrap();
        let got: Vec<_> = segs
            .iter()
            .map(|s| (s.start_ms, s.end_ms, s.text.as_str(), s.speaker.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![
                (0, 1520, "Xin chào.", Some("1")),
                (1520, 3000, "Hello there", Some("0")),
                (3500, 4000, "Dạ.", Some("1")),
            ]
        );
        let req = reqs.recv().unwrap();
        assert!(req.starts_with(
            "POST /v1/listen?model=nova-3&language=multi&diarize_model=latest&mip_opt_out=true&utterances=true"
        ));
        assert!(req
            .to_lowercase()
            .contains("authorization: token sekret-key"));
        assert!(req.contains("RIFF") && req.contains("WAVEfmt "));
        assert!(req.contains("mip_opt_out=true"));
        assert!(
            !req.contains("diarize=true"),
            "deprecated, and rejected together with diarize_model"
        );
    }

    #[test]
    fn a_429_is_retried_then_succeeds() {
        let (url, reqs) = mock(vec![
            (429, "Retry-After: 0\r\n", "slow down".into()),
            (503, "", "busy".into()),
            (200, "", OK.into()),
        ]);
        assert_eq!(client(&url).transcribe(&[0.0; 160], "en").unwrap().len(), 3);
        assert_eq!(reqs.try_iter().count(), 3);
    }

    #[test]
    fn a_401_is_not_retried_and_the_error_holds_no_key_or_body() {
        let (url, reqs) = mock(vec![
            (401, "", r#"{"err":"bad sekret-key Xin chào"}"#.into()),
            (200, "", OK.into()),
        ]);
        let e = client(&url)
            .transcribe(&[0.0; 160], "vi")
            .unwrap_err()
            .to_string();
        assert_eq!(reqs.try_iter().count(), 1, "no retry on 401");
        assert!(e.contains("401"), "{e}");
        assert!(!e.contains("sekret-key") && !e.contains("chào"), "{e}");
    }

    #[test]
    fn retries_stop_after_two_and_the_error_says_unavailable() {
        let (url, reqs) = mock(vec![
            (500, "", "x".into()),
            (500, "", "x".into()),
            (500, "", "x".into()),
        ]);
        let e = client(&url)
            .transcribe(&[0.0; 160], "en")
            .unwrap_err()
            .to_string();
        assert_eq!(reqs.try_iter().count(), 3);
        assert!(e.contains("unavailable") && e.contains("500"), "{e}");
    }

    #[test]
    fn the_meeting_language_is_sent_explicitly_and_an_unknown_one_is_refused() {
        for (lang, sent) in [
            ("vi", "vi"),
            ("en", "en"),
            ("multi", "multi"),
            ("vi+en", "multi"),
            ("auto", "multi"),
        ] {
            let (url, reqs) = mock(vec![(200, "", OK.into())]);
            client(&url).transcribe(&[0.0; 160], lang).unwrap();
            let req = reqs.recv().unwrap();
            assert!(
                req.starts_with(&format!("POST /v1/listen?model=nova-3&language={sent}&")),
                "{lang}: {req}"
            );
        }
        // no request is made for an unknown language
        let (url, reqs) = mock(vec![(200, "", OK.into())]);
        let e = client(&url)
            .transcribe(&[0.0; 160], "fr")
            .unwrap_err()
            .to_string();
        assert!(e.contains("unsupported meeting language"), "{e}");
        assert!(reqs.try_recv().is_err());
    }

    #[test]
    fn wav_round_trips_through_the_decoder() {
        let samples: Vec<f32> = (0..3200)
            .map(|i| ((i as f32) / 3200.0 * 2.0 - 1.0) * 0.9)
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        std::fs::write(&p, encode_wav(&samples)).unwrap();
        let back = crate::decode::decode_to_16k_mono(&p).unwrap();
        assert_eq!(back.len(), samples.len());
        assert!(samples
            .iter()
            .zip(&back)
            .all(|(a, b)| (a - b).abs() < 1.0 / 16384.0));
    }
}
