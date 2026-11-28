//! Deepgram streaming: sends the notetaker's 16 kHz mono i16 frames over a WebSocket and yields
//! partial and final text; each final line carries the provider's speaker number. Never blocks or fails the caller: any trouble ends in
//! one `LiveEvent::Unavailable`, and later frames are dropped. Errors and logs carry no key and no text.
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::{client::IntoClientRequest, stream::MaybeTlsStream, Message};

pub const DEEPGRAM_LISTEN: &str = "wss://api.deepgram.com/v1/listen";
/// About 10 s of 100 ms frames; past that the link is stuck and frames are dropped, not queued.
const BACKLOG: usize = 100;
/// How long a finished stream waits for the provider's last results.
const FLUSH: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq)]
pub enum LiveEvent {
    /// Text that may still change.
    Partial(String),
    /// Text the provider will not change. `speaker` is its 0-based speaker number (`None` when the provider
    /// gave none) and `ms` how long the words took to say.
    Final {
        text: String,
        speaker: Option<u32>,
        ms: i64,
    },
    /// The connection failed or dropped. Recording goes on; the live view says so.
    Unavailable,
}

pub struct DeepgramStream {
    frames: SyncSender<Vec<i16>>,
}

impl DeepgramStream {
    /// Returns at once; the connection is made on a worker thread. `language` is a hint such as "vi".
    pub fn start(base_url: &str, api_key: &str, language: &str) -> (Self, Receiver<LiveEvent>) {
        let (ftx, frx) = mpsc::sync_channel(BACKLOG);
        let (etx, erx) = mpsc::channel();
        let Some(lang) = crate::deepgram::deepgram_language(language) else {
            // An unknown language never reaches the provider: the live view just says unavailable.
            drop(frx);
            let _ = etx.send(LiveEvent::Unavailable);
            return (Self { frames: ftx }, erx);
        };
        let url = format!(
            "{base_url}?model=nova-3&language={lang}&encoding=linear16&sample_rate=16000&channels=1&diarize=true&interim_results=true&punctuate=true&smart_format=true&endpointing=400&utterance_end_ms=1000&mip_opt_out=true"
        );
        let key = api_key.to_string();
        thread::spawn(move || {
            let failed = run(&url, &key, &frx, &etx).is_err();
            // Closed first, so frames are refused from the moment the event is seen.
            drop(frx);
            if failed {
                let _ = etx.send(LiveEvent::Unavailable);
            }
        });
        (Self { frames: ftx }, erx)
    }

    /// Queues one frame. Never blocks; returns false when the frame was dropped.
    pub fn send_frame(&self, samples: &[i16]) -> bool {
        self.frames.try_send(samples.to_vec()).is_ok()
    }
}

fn run(
    url: &str,
    key: &str,
    frames: &Receiver<Vec<i16>>,
    events: &Sender<LiveEvent>,
) -> Result<(), ()> {
    let mut req = url.into_client_request().map_err(|_| ())?;
    req.headers_mut().insert(
        "Authorization",
        format!("Token {key}").parse().map_err(|_| ())?,
    );
    let (mut ws, _) = tungstenite::connect(req).map_err(|_| ())?;
    let mut pending = Pending::default();
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => set_poll(s),
        MaybeTlsStream::Rustls(s) => set_poll(s.get_mut()),
        _ => Err(()),
    }?;
    loop {
        loop {
            match frames.try_recv() {
                Ok(f) => {
                    let bytes: Vec<u8> = f.iter().flat_map(|s| s.to_le_bytes()).collect();
                    ws.send(Message::Binary(bytes.into())).map_err(|_| ())?;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // The caller is done: ask the provider to flush, then leave.
                    let _ = ws.send(Message::Text(r#"{"type":"CloseStream"}"#.into()));
                    // Collect the last results until the provider closes, for at most FLUSH.
                    let end = Instant::now() + FLUSH;
                    while Instant::now() < end {
                        match ws.read() {
                            Ok(Message::Text(t)) => {
                                handle(&t, &mut pending, events);
                            }
                            Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => {
                                break
                            }
                            Ok(_) => {}
                            Err(tungstenite::Error::Io(e))
                                if matches!(
                                    e.kind(),
                                    ErrorKind::WouldBlock | ErrorKind::TimedOut
                                ) => {}
                            Err(_) => break,
                        }
                    }
                    flush(&mut pending, events);
                    let _ = ws.close(None);
                    return Ok(());
                }
            }
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                handle(&t, &mut pending, events);
            }
            Ok(Message::Close(_)) => return Err(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return Err(()),
        }
    }
}

fn set_poll(s: &TcpStream) -> Result<(), ()> {
    s.set_read_timeout(Some(POLL)).map_err(|_| ())
}

/// Finished pieces of the line being built, and who is speaking them.
#[derive(Default)]
struct Pending {
    text: String,
    speaker: Option<u32>,
    ms: i64,
}

/// Sends the buffered finished pieces as one `Final` line.
fn flush(pending: &mut Pending, events: &Sender<LiveEvent>) {
    let p = std::mem::take(pending);
    if !p.text.is_empty() {
        let _ = events.send(LiveEvent::Final {
            text: p.text,
            speaker: p.speaker,
            ms: p.ms,
        });
    }
}

/// Turns one provider message into events. The provider finalises many short pieces per sentence;
/// they are held in `pending` and sent as one `Final` when it reports the speaker paused
/// (`speech_final`) or the utterance ended, so a line is a whole sentence, not a fragment. A piece by a
/// different speaker than the line so far ends that line first. Everything else (metadata, empty audio)
/// is ignored.
fn handle(msg: &str, pending: &mut Pending, events: &Sender<LiveEvent>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(msg) else {
        return;
    };
    match v["type"].as_str() {
        Some("UtteranceEnd") => flush(pending, events),
        Some("Results") => {
            let text = v["channel"]["alternatives"][0]["transcript"]
                .as_str()
                .unwrap_or("")
                .trim();
            if v["is_final"].as_bool() == Some(true) {
                if !text.is_empty() {
                    // The speaker of a piece is the one of its first word.
                    let speaker = v["channel"]["alternatives"][0]["words"][0]["speaker"]
                        .as_u64()
                        .map(|n| n as u32);
                    if pending.speaker.is_some() && speaker.is_some() && pending.speaker != speaker
                    {
                        flush(pending, events);
                    }
                    if pending.text.is_empty() {
                        pending.speaker = speaker;
                    } else {
                        pending.text.push(' ');
                    }
                    pending.text.push_str(text);
                    pending.ms += (v["duration"].as_f64().unwrap_or(0.0) * 1000.0).round() as i64;
                }
                if v["speech_final"].as_bool() == Some(true) {
                    flush(pending, events);
                } else if !pending.text.is_empty() {
                    let _ = events.send(LiveEvent::Partial(pending.text.clone()));
                }
            } else if !text.is_empty() {
                let shown = if pending.text.is_empty() {
                    text.to_string()
                } else {
                    format!("{} {text}", pending.text)
                };
                let _ = events.send(LiveEvent::Partial(shown));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[allow(clippy::result_large_err)] // the mock's handshake callback type is tungstenite's
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc::RecvTimeoutError;

    const WAIT: Duration = Duration::from_secs(5);

    fn fin(text: &str) -> LiveEvent {
        LiveEvent::Final {
            text: text.into(),
            speaker: None,
            ms: 0,
        }
    }

    fn results(text: &str, fin: bool) -> Message {
        format!(r#"{{"type":"Results","is_final":{fin},"speech_final":{fin},"channel":{{"alternatives":[{{"transcript":"{text}"}}]}}}}"#).into()
    }

    /// Mock provider on localhost. `script` gets the accepted socket and the Authorization header.
    fn mock(
        script: impl FnOnce(tungstenite::WebSocket<TcpStream>, String, String) + Send + 'static,
    ) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}/v1/listen", l.local_addr().unwrap());
        thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let (mut auth, mut uri) = (String::new(), String::new());
            let ws =
                tungstenite::accept_hdr(s, |req: &tungstenite::handshake::server::Request, r| {
                    uri = req.uri().to_string();
                    auth = req.headers()["authorization"].to_str().unwrap().to_string();
                    Ok(r)
                })
                .unwrap();
            script(ws, auth, uri);
        });
        url
    }

    #[test]
    fn frames_go_out_as_le_bytes_and_text_comes_back() {
        let (got_tx, got_rx) = mpsc::channel();
        let url = mock(move |mut ws, auth, uri| {
            let Message::Binary(b) = ws.read().unwrap() else {
                panic!("not binary")
            };
            got_tx.send((auth, uri, b.to_vec())).unwrap();
            ws.send(results("xin", false)).unwrap();
            ws.send(results("xin chào", true)).unwrap();
            ws.send(results("", true)).unwrap(); // empty text is not an event
            ws.send(Message::Text(r#"{"type":"Metadata"}"#.into()))
                .unwrap();
            let Message::Text(t) = ws.read().unwrap() else {
                panic!("not text")
            };
            assert!(t.contains("CloseStream"));
            ws.send(results("hết", true)).unwrap(); // the flushed tail
            let _ = ws.close(None);
        });
        let (s, events) = DeepgramStream::start(&url, "k3y", "vi+en");
        assert!(s.send_frame(&[1, -2, 256]));
        let (auth, uri, bytes) = got_rx.recv_timeout(WAIT).unwrap();
        assert_eq!(auth, "Token k3y");
        assert!(
            uri.contains("mip_opt_out=true"),
            "opted out of model improvement: {uri}"
        );
        for p in [
            "diarize=true",
            "punctuate=true",
            "smart_format=true",
            "endpointing=400",
            "utterance_end_ms=1000",
        ] {
            assert!(uri.contains(p), "{p} missing: {uri}");
        }
        assert_eq!(bytes, vec![1, 0, 254, 255, 0, 1]);
        assert_eq!(
            events.recv_timeout(WAIT).unwrap(),
            LiveEvent::Partial("xin".into())
        );
        assert_eq!(events.recv_timeout(WAIT).unwrap(), fin("xin chào"));
        drop(s);
        // the flushed tail arrives after the drop, then the channel ends without Unavailable
        assert_eq!(events.recv_timeout(WAIT).unwrap(), fin("hết"));
        assert_eq!(
            events.recv_timeout(WAIT),
            Err(RecvTimeoutError::Disconnected)
        );
    }

    fn msg(text: &str, fin: bool, speech_final: bool) -> String {
        format!(
            r#"{{"type":"Results","is_final":{fin},"speech_final":{speech_final},"channel":{{"alternatives":[{{"transcript":"{text}"}}]}}}}"#
        )
    }

    #[test]
    fn finalised_pieces_make_one_line_until_the_speaker_pauses() {
        let (tx, rx) = mpsc::channel();
        let mut pending = Pending::default();
        for m in [
            msg("Xin chào", false, false),
            msg("Xin chào,", true, false),
            msg("mọi người", false, false),
            msg("mọi người.", true, true),
            msg("Tiếp", true, false),
            r#"{"type":"UtteranceEnd"}"#.to_string(),
            r#"{"type":"UtteranceEnd"}"#.to_string(), // nothing left to flush
        ] {
            handle(&m, &mut pending, &tx);
        }
        drop(tx);
        let got: Vec<_> = rx.iter().collect();
        assert_eq!(
            got,
            vec![
                LiveEvent::Partial("Xin chào".into()),
                LiveEvent::Partial("Xin chào,".into()),
                LiveEvent::Partial("Xin chào, mọi người".into()),
                fin("Xin chào, mọi người."),
                LiveEvent::Partial("Tiếp".into()),
                fin("Tiếp"),
            ]
        );
        assert!(pending.text.is_empty());
    }

    /// A finished piece spoken by `speaker` for `secs` seconds.
    fn said(text: &str, speaker: u32, secs: f64, speech_final: bool) -> String {
        format!(
            r#"{{"type":"Results","is_final":true,"speech_final":{speech_final},"duration":{secs},"channel":{{"alternatives":[{{"transcript":"{text}","words":[{{"word":"x","speaker":{speaker}}}]}}]}}}}"#
        )
    }

    #[test]
    fn a_new_speaker_ends_the_line_and_each_line_carries_who_and_how_long() {
        let (tx, rx) = mpsc::channel();
        let mut pending = Pending::default();
        for m in [
            said("Hello", 0, 0.5, false),
            said("there.", 0, 0.75, false),
            said("Hi!", 1, 0.25, true),
            said("Bye.", 0, 1.0, true),
        ] {
            handle(&m, &mut pending, &tx);
        }
        drop(tx);
        let line = |text: &str, speaker, ms| LiveEvent::Final {
            text: text.into(),
            speaker: Some(speaker),
            ms,
        };
        let got: Vec<_> = rx.iter().collect();
        assert_eq!(
            got,
            vec![
                LiveEvent::Partial("Hello".into()),
                LiveEvent::Partial("Hello there.".into()),
                line("Hello there.", 0, 1250),
                line("Hi!", 1, 250),
                line("Bye.", 0, 1000),
            ]
        );
    }

    #[test]
    fn a_dropped_connection_reports_unavailable_and_sending_never_blocks() {
        let url = mock(|mut ws, _, _| {
            let _ = ws.read();
            // dropped without a close handshake
        });
        let (s, events) = DeepgramStream::start(&url, "k", "en");
        s.send_frame(&[0; 8]);
        assert_eq!(events.recv_timeout(WAIT).unwrap(), LiveEvent::Unavailable);
        // The worker is gone: every later frame is dropped, and none of the calls blocks.
        let accepted = (0..BACKLOG + 10).filter(|_| s.send_frame(&[0; 8])).count();
        assert_eq!(accepted, 0);
    }

    #[test]
    fn a_refused_connection_reports_unavailable() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}/", l.local_addr().unwrap());
        drop(l);
        let (_s, events) = DeepgramStream::start(&url, "k", "en");
        assert_eq!(events.recv_timeout(WAIT).unwrap(), LiveEvent::Unavailable);
    }

    #[test]
    fn the_language_is_in_the_url_and_an_unknown_one_is_unavailable_at_once() {
        let (uri_tx, uri_rx) = mpsc::channel();
        let url = mock(move |mut ws, _, uri| {
            uri_tx.send(uri).unwrap();
            let _ = ws.read();
        });
        let (s, _events) = DeepgramStream::start(&url, "k", "vi");
        s.send_frame(&[0; 4]);
        assert!(uri_rx.recv_timeout(WAIT).unwrap().contains("language=vi&"));
        let (s, events) = DeepgramStream::start("ws://127.0.0.1:1/never-used", "k", "fr");
        assert_eq!(events.recv_timeout(WAIT).unwrap(), LiveEvent::Unavailable);
        assert!(!s.send_frame(&[0; 4]));
    }
}
