//! Deepgram streaming: sends the notetaker's 16 kHz mono i16 frames over a WebSocket and yields
//! partial and final text, without speakers. Never blocks or fails the caller: any trouble ends in
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
    /// Text the provider will not change.
    Final(String),
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
        // Deepgram's multilingual model covers mixed vi+en; "auto" and "vi+en" both use it.
        let lang = match language {
            "vi" | "en" => language,
            _ => "multi",
        };
        let url = format!(
            "{base_url}?model=nova-3&language={lang}&encoding=linear16&sample_rate=16000&channels=1&interim_results=true"
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
                                if let Some(ev) = parse(&t) {
                                    let _ = events.send(ev);
                                }
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
                    let _ = ws.close(None);
                    return Ok(());
                }
            }
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                if let Some(ev) = parse(&t) {
                    let _ = events.send(ev);
                }
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

/// A `Results` message with non-empty text; everything else (metadata, empty audio) is ignored.
fn parse(msg: &str) -> Option<LiveEvent> {
    let v: serde_json::Value = serde_json::from_str(msg).ok()?;
    if v["type"] != "Results" {
        return None;
    }
    let text = v["channel"]["alternatives"][0]["transcript"]
        .as_str()?
        .trim();
    if text.is_empty() {
        return None;
    }
    let text = text.to_string();
    Some(if v["is_final"].as_bool() == Some(true) {
        LiveEvent::Final(text)
    } else {
        LiveEvent::Partial(text)
    })
}

#[cfg(test)]
#[allow(clippy::result_large_err)] // the mock's handshake callback type is tungstenite's
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc::RecvTimeoutError;

    const WAIT: Duration = Duration::from_secs(5);

    fn results(text: &str, fin: bool) -> Message {
        format!(r#"{{"type":"Results","is_final":{fin},"channel":{{"alternatives":[{{"transcript":"{text}"}}]}}}}"#).into()
    }

    /// Mock provider on localhost. `script` gets the accepted socket and the Authorization header.
    fn mock(
        script: impl FnOnce(tungstenite::WebSocket<TcpStream>, String) + Send + 'static,
    ) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}/v1/listen", l.local_addr().unwrap());
        thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut auth = String::new();
            let ws =
                tungstenite::accept_hdr(s, |req: &tungstenite::handshake::server::Request, r| {
                    auth = req.headers()["authorization"].to_str().unwrap().to_string();
                    Ok(r)
                })
                .unwrap();
            script(ws, auth);
        });
        url
    }

    #[test]
    fn frames_go_out_as_le_bytes_and_text_comes_back() {
        let (got_tx, got_rx) = mpsc::channel();
        let url = mock(move |mut ws, auth| {
            let Message::Binary(b) = ws.read().unwrap() else {
                panic!("not binary")
            };
            got_tx.send((auth, b.to_vec())).unwrap();
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
        let (auth, bytes) = got_rx.recv_timeout(WAIT).unwrap();
        assert_eq!(auth, "Token k3y");
        assert_eq!(bytes, vec![1, 0, 254, 255, 0, 1]);
        assert_eq!(
            events.recv_timeout(WAIT).unwrap(),
            LiveEvent::Partial("xin".into())
        );
        assert_eq!(
            events.recv_timeout(WAIT).unwrap(),
            LiveEvent::Final("xin chào".into())
        );
        drop(s);
        // closing sends no Unavailable and no further text
        assert_eq!(
            events.recv_timeout(WAIT),
            Err(RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn a_dropped_connection_reports_unavailable_and_sending_never_blocks() {
        let url = mock(|mut ws, _| {
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
}
