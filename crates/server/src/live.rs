//! The live transcript of calls in progress: in memory only, never written to disk or logged, and
//! dropped when the call ends or someone opts out. The notetaker writes into it, the live view reads.
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};
use tokio::sync::broadcast;

/// Most recent finished lines kept per call.
// ponytail: a count, not an age; the Ask window is 60 s, so this is far more than it needs.
const MAX_LINES: usize = 2000;

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// Text that may still change; replaces the previous partial.
    Partial(String),
    /// A finished line, `at_ms` after the call was admitted.
    Final { at_ms: i64, text: String },
    /// Streaming failed; recording goes on.
    Unavailable,
    /// The call ended or someone opted out.
    End,
}

struct Call {
    lines: Vec<(i64, String)>,
    partial: String,
    unavailable: bool,
    tx: broadcast::Sender<Msg>,
}

/// What a viewer gets on connecting: the lines so far, then everything new.
pub struct Snapshot {
    pub lines: Vec<(i64, String)>,
    pub partial: String,
    pub unavailable: bool,
    pub rx: broadcast::Receiver<Msg>,
}

#[derive(Default)]
pub struct LiveHub {
    calls: Mutex<HashMap<String, Call>>,
}

impl LiveHub {
    pub fn start(&self, id: &str) {
        let (tx, _) = broadcast::channel(256);
        self.calls.lock().unwrap().insert(
            id.into(),
            Call {
                lines: Vec::new(),
                partial: String::new(),
                unavailable: false,
                tx,
            },
        );
    }

    pub fn push(&self, id: &str, m: Msg) {
        let mut calls = self.calls.lock().unwrap();
        let Some(c) = calls.get_mut(id) else { return };
        match &m {
            Msg::Partial(t) => c.partial = t.clone(),
            Msg::Final { at_ms, text } => {
                c.partial.clear();
                c.lines.push((*at_ms, text.clone()));
                if c.lines.len() > MAX_LINES {
                    c.lines.remove(0);
                }
            }
            Msg::Unavailable => c.unavailable = true,
            Msg::End => {}
        }
        let _ = c.tx.send(m);
    }

    /// Drops the transcript and tells viewers the call is over.
    pub fn clear(&self, id: &str) {
        if let Some(c) = self.calls.lock().unwrap().remove(id) {
            let _ = c.tx.send(Msg::End);
        }
    }

    /// `None` when the call is not live here.
    pub fn subscribe(&self, id: &str) -> Option<Snapshot> {
        let calls = self.calls.lock().unwrap();
        let c = calls.get(id)?;
        Some(Snapshot {
            lines: c.lines.clone(),
            partial: c.partial.clone(),
            unavailable: c.unavailable,
            rx: c.tx.subscribe(),
        })
    }
}

// ponytail: one hub per process, as the notetaker and the HTTP server share one; pass it in if they split.
pub fn hub() -> &'static LiveHub {
    static HUB: OnceLock<LiveHub> = OnceLock::new();
    HUB.get_or_init(LiveHub::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_late_viewer_gets_the_finished_lines_and_the_current_partial() {
        let h = LiveHub::default();
        h.start("m");
        h.push(
            "m",
            Msg::Final {
                at_ms: 100,
                text: "hello".into(),
            },
        );
        h.push("m", Msg::Partial("wor".into()));
        let s = h.subscribe("m").unwrap();
        assert_eq!(s.lines, vec![(100, "hello".to_string())]);
        assert_eq!(s.partial, "wor");
        h.push(
            "m",
            Msg::Final {
                at_ms: 200,
                text: "world".into(),
            },
        );
        assert_eq!(h.subscribe("m").unwrap().partial, "");
    }

    #[test]
    fn clear_drops_the_transcript_and_tells_viewers() {
        let h = LiveHub::default();
        h.start("m");
        let mut s = h.subscribe("m").unwrap();
        h.push(
            "m",
            Msg::Final {
                at_ms: 1,
                text: "secret".into(),
            },
        );
        h.clear("m");
        assert!(h.subscribe("m").is_none());
        assert_eq!(
            s.rx.try_recv().unwrap(),
            Msg::Final {
                at_ms: 1,
                text: "secret".into()
            }
        );
        assert_eq!(s.rx.try_recv().unwrap(), Msg::End);
        h.push(
            "m",
            Msg::Final {
                at_ms: 2,
                text: "late".into(),
            },
        );
        assert!(
            h.subscribe("m").is_none(),
            "a push after clear keeps nothing"
        );
    }

    #[test]
    fn the_line_buffer_is_bounded() {
        let h = LiveHub::default();
        h.start("m");
        for i in 0..(MAX_LINES as i64 + 5) {
            h.push(
                "m",
                Msg::Final {
                    at_ms: i,
                    text: "x".into(),
                },
            );
        }
        let s = h.subscribe("m").unwrap();
        assert_eq!((s.lines.len(), s.lines[0].0), (MAX_LINES, 5));
    }
}
