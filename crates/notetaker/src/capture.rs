//! Receives the call audio that `meet/capture.js` mixes in the page: 16 kHz mono, about 100 ms per
//! frame. Install it on a page before the call page loads, then read frames from the channel.
use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use chromiumoxide::{
    cdp::js_protocol::runtime::{AddBindingParams, EventBindingCalled},
    Page,
};
use futures_util::StreamExt;
use tokio::{sync::mpsc, task::JoinHandle};

const SCRIPT: &str = include_str!("meet/capture.js");
const BINDING: &str = "recapzAudio";
/// A real frame is 3200 bytes, 4268 base64 characters. Anything larger comes from a page that is
/// not our script (any script on the call page can call the binding) and is dropped unread.
const MAX_PAYLOAD: usize = 8 * 1024;

pub struct AudioCapture {
    pub frames: mpsc::Receiver<Vec<i16>>,
    task: JoinHandle<()>,
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Decodes one binding payload: base64 of 16-bit little-endian samples. Bad payloads give `None`.
pub fn decode_frame(payload: &str) -> Option<Vec<i16>> {
    if payload.len() > MAX_PAYLOAD {
        return None;
    }
    let bytes = STANDARD.decode(payload).ok()?;
    if bytes.len() % 2 != 0 {
        return None;
    }
    let (pairs, _) = bytes.as_chunks::<2>();
    Some(pairs.iter().map(|b| i16::from_le_bytes(*b)).collect())
}

/// What the capture script saw so far: `peers=2 tracks=3 frames=120 path=worklet`. Counts and one
/// word only, so it is safe to log.
pub async fn stats(page: &Page) -> Option<String> {
    let v = page
        .evaluate("JSON.stringify(window.__recapzCapture ?? null)")
        .await
        .ok()?
        .into_value::<String>()
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&v).ok()?;
    Some(format!(
        "peers={} tracks={} frames={} path={}",
        v["peers"],
        v["tracks"],
        v["frames"],
        v["path"].as_str()?
    ))
}

/// Adds the capture script to every document the page loads from now on (and the current one),
/// and starts forwarding its frames.
pub async fn start(page: &Page) -> Result<AudioCapture> {
    let mut events = page.event_listener::<EventBindingCalled>().await?;
    page.execute(AddBindingParams::new(BINDING)).await?;
    page.evaluate_on_new_document(SCRIPT).await?;
    page.evaluate(SCRIPT).await?;
    let (tx, frames) = mpsc::channel(256);
    let task = tokio::spawn(async move {
        while let Some(e) = events.next().await {
            if e.name != BINDING {
                continue;
            }
            if let Some(frame) = decode_frame(&e.payload) {
                // Never wait on the reader: a flood from the page must not stall or grow the queue.
                if matches!(
                    tx.try_send(frame),
                    Err(mpsc::error::TrySendError::Closed(_))
                ) {
                    break; // the reader is gone
                }
            }
        }
    });
    Ok(AudioCapture { frames, task })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_decode_as_little_endian_samples_and_bad_payloads_are_dropped() {
        // 0x0100 = 256, 0xFFFF = -1, 0x7FFF = 32767
        let payload = STANDARD.encode([0x00, 0x01, 0xFF, 0xFF, 0xFF, 0x7F]);
        assert_eq!(decode_frame(&payload), Some(vec![256, -1, 32767]));
        assert_eq!(decode_frame(""), Some(vec![]));
        assert_eq!(decode_frame("not base64!"), None);
        assert_eq!(
            decode_frame(&STANDARD.encode([1, 2, 3])),
            None,
            "odd byte count"
        );
        let real = STANDARD.encode(vec![0u8; 3200]);
        assert_eq!(real.len(), 4268);
        assert_eq!(decode_frame(&real).map(|f| f.len()), Some(1600));
        let just_over = "A".repeat(MAX_PAYLOAD + 4); // valid base64, one group over the cap
        assert_eq!(decode_frame(&just_over), None);
        assert_eq!(decode_frame(&"A".repeat(100_000)), None, "dropped unread");
    }
}

/// Real Chromium, a local page, and an in-page WebRTC loopback playing a 440 Hz tone. Run with
/// `make test-browser`; prints SKIPPED and returns when no Chromium is installed.
#[cfg(all(test, feature = "browser-tests"))]
mod browser_tests {
    use super::*;
    use crate::{
        browser::{find_chromium, Browser, Launch},
        wav::WavWriter,
    };
    use std::{
        io::Cursor,
        time::{Duration, Instant},
    };

    macro_rules! fixture {
        ($attr:literal) => {
            concat!(
                r#"<!doctype html><title>tone</title><script"#,
                $attr,
                r#">
    try { eval('window.__e = 1'); } catch (e) { window.__e = e.name; }
    (async () => {
      const a = new AudioContext(), osc = a.createOscillator(), dest = a.createMediaStreamDestination();
      osc.frequency.value = 440; osc.connect(dest); osc.start();
      const pc1 = new RTCPeerConnection(), pc2 = new RTCPeerConnection();
      pc1.onicecandidate = (e) => e.candidate && pc2.addIceCandidate(e.candidate);
      pc2.onicecandidate = (e) => e.candidate && pc1.addIceCandidate(e.candidate);
      dest.stream.getTracks().forEach((t) => pc1.addTrack(t, dest.stream));
      await pc1.setLocalDescription(await pc1.createOffer());
      await pc2.setRemoteDescription(pc1.localDescription);
      await pc2.setLocalDescription(await pc2.createAnswer());
      await pc1.setRemoteDescription(pc2.localDescription);
    })();
    </script>"#
            )
        };
    }
    const FIXTURE: &str = fixture!("");
    /// Carries the nonce of the policy below, like the call page's own scripts do.
    const NONCED: &str = fixture!(r#" nonce="n1""#);
    /// The shape of the call page's policy: only nonced scripts run, nothing else loads a script,
    /// and strings may not become scripts.
    const STRICT: &str = "Content-Security-Policy: script-src 'nonce-n1' 'strict-dynamic'; require-trusted-types-for 'script'\r\n";

    /// Signal power at one frequency (Goertzel).
    fn power(samples: &[i16], hz: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * hz / 16_000.0;
        let k = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0, 0.0);
        for &x in samples {
            let s = x as f64 + k * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        s1 * s1 + s2 * s2 - k * s1 * s2
    }

    /// Opens `url` with capture installed and returns three seconds of samples plus the capture's
    /// own stats. `refuse_worklet` makes loading a worklet fail, as a page policy could.
    async fn capture_tone(url: String, refuse_worklet: bool) -> Option<(Vec<i16>, String)> {
        if find_chromium(None).is_none() {
            println!("SKIPPED: no Chromium found");
            return None;
        }
        let work = std::env::temp_dir().join(format!(
            "recapz-capture-{}-{refuse_worklet}-{}",
            std::process::id(),
            url.len()
        ));
        let _ = std::fs::remove_dir_all(&work);
        let b = Browser::launch(Launch {
            chromium: None,
            profile: None,
            work_dir: work,
            headless: true,
        })
        .await
        .unwrap();
        let page = b.inner.new_page("about:blank").await.unwrap();
        if refuse_worklet {
            page.evaluate_on_new_document(
                "AudioWorklet.prototype.addModule = () => Promise.reject(new DOMException('refused', 'SecurityError'))",
            )
            .await
            .unwrap();
        }
        let mut cap = start(&page).await.unwrap();
        page.goto(url).await.unwrap();

        // Collect three seconds of frames, starting at the first one.
        let mut samples: Vec<i16> = Vec::new();
        let first = tokio::time::timeout(Duration::from_secs(15), cap.frames.recv())
            .await
            .expect("no audio frame within 15 s")
            .unwrap();
        let began = Instant::now();
        samples.extend(first);
        while began.elapsed() < Duration::from_secs(3) {
            let f = tokio::time::timeout(Duration::from_secs(2), cap.frames.recv())
                .await
                .expect("frames stopped")
                .unwrap();
            samples.extend(f);
        }
        let wall_ms = began.elapsed().as_millis() as i64;
        let audio_ms = samples.len() as i64 / 16;
        assert!(
            (audio_ms - wall_ms).abs() <= 200,
            "audio {audio_ms} ms vs wall {wall_ms} ms"
        );
        let st = stats(&page).await.expect("capture stats");
        let eval_result = page
            .evaluate("String(window.__e)")
            .await
            .unwrap()
            .into_value::<String>()
            .unwrap();
        Some((samples, format!("{st} eval={eval_result}")))
    }

    fn assert_440_dominates(samples: &[i16]) {
        // After the first second, the 440 Hz bin dominates the others by far.
        let tail = &samples[16_000..];
        let at = |hz| power(tail, hz);
        for other in [220.0, 300.0, 600.0, 1000.0, 2000.0] {
            assert!(
                at(440.0) > 20.0 * at(other),
                "440 Hz not dominant over {other} Hz"
            );
        }
    }

    #[tokio::test]
    async fn captures_a_tone_played_over_an_in_page_connection() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((samples, st)) = capture_tone(crate::test_support::serve(FIXTURE), false).await
        else {
            return;
        };
        assert_440_dominates(&samples);
        assert!(st.starts_with("peers=2 tracks=1 frames="), "{st}");
        assert!(
            st.contains("path=worklet eval=1"),
            "{st}: no policy, eval works"
        );
        // The frames make a valid recording of the same length.
        let mut w = WavWriter::new(Cursor::new(Vec::new())).unwrap();
        w.write(&samples).unwrap();
        let wav = w.finish().unwrap().into_inner();
        assert_eq!(wav.len(), 44 + samples.len() * 2);
    }

    /// A page policy shaped like the call page's (nonces, strict-dynamic, trusted types) is in force
    /// (the tests prove it by `eval` being refused) and the worklet still loads from a blob.
    #[tokio::test]
    async fn captures_on_a_page_with_a_strict_script_policy() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let url = crate::test_support::serve_with(NONCED, STRICT);
        let Some((samples, st)) = capture_tone(url, false).await else {
            return;
        };
        assert_440_dominates(&samples);
        assert!(
            st.ends_with("path=worklet eval=EvalError"),
            "{st}: the policy must be in force"
        );
    }

    /// If the worklet is refused, the script processor still delivers the audio, and the stats say
    /// which path was used and why.
    #[tokio::test]
    async fn falls_back_to_a_script_processor_when_the_worklet_is_refused() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let url = crate::test_support::serve(FIXTURE);
        let Some((samples, st)) = capture_tone(url, true).await else {
            return;
        };
        assert_440_dominates(&samples);
        assert!(st.contains("path=processor: SecurityError"), "{st}");
    }
}
