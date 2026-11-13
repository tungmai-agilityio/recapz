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
        io::{Cursor, Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };

    const FIXTURE: &str = r#"<!doctype html><title>tone</title><script>
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
    </script>"#;

    /// Serves the fixture on localhost (a secure context, which audio worklets require).
    fn serve() -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut c in l.incoming().flatten() {
                let mut buf = [0u8; 1024];
                let _ = c.read(&mut buf);
                let _ = write!(
                    c,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{FIXTURE}",
                    FIXTURE.len()
                );
            }
        });
        url
    }

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

    #[tokio::test]
    async fn captures_a_tone_played_over_an_in_page_connection() {
        if find_chromium(None).is_none() {
            println!("SKIPPED: no Chromium found");
            return;
        }
        let work = std::env::temp_dir().join(format!("recapz-capture-{}", std::process::id()));
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
        let mut cap = start(&page).await.unwrap();
        page.goto(serve()).await.unwrap();

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

        // The tone is 440 Hz: after the first second, that bin dominates the others by far.
        let tail = &samples[16_000..];
        let at = |hz| power(tail, hz);
        for other in [220.0, 300.0, 600.0, 1000.0, 2000.0] {
            assert!(
                at(440.0) > 20.0 * at(other),
                "440 Hz not dominant over {other} Hz"
            );
        }

        // The frames make a valid recording of the same length.
        let mut w = WavWriter::new(Cursor::new(Vec::new())).unwrap();
        w.write(&samples).unwrap();
        let wav = w.finish().unwrap().into_inner();
        assert_eq!(wav.len(), 44 + samples.len() * 2);
    }
}
