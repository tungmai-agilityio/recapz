//! The live transcript of calls in progress: in memory only, never written to disk or logged, and
//! dropped when the call ends or someone opts out. The notetaker writes into it, the live view reads.
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Instant,
};
use tokio::sync::broadcast;

/// Most recent finished lines kept per call.
// ponytail: a count, not an age; the Ask window is 60 s, so this is far more than it needs.
const MAX_LINES: usize = 2000;

/// Who said a line: the provider's 0-based speaker number (one colour per number), the name or
/// "Speaker N" to show for it, and how long the words took in ms.
#[derive(Debug, Clone, PartialEq)]
pub struct Who {
    pub n: u32,
    pub label: String,
    pub ms: i64,
}

/// Names the provider's speaker numbers from the notetaker's active-speaker samples: each finished
/// line votes for the name that was active when it arrived, and a number shows its most voted name,
/// or "Speaker N" (1-based) while it has none. Browser and desktop recordings have no samples.
#[derive(Default)]
pub struct Labels(HashMap<u32, HashMap<String, u32>>);

impl Labels {
    pub fn label(&mut self, n: u32, active: Option<&str>) -> String {
        let votes = self.0.entry(n).or_default();
        if let Some(a) = active {
            *votes.entry(a.to_string()).or_default() += 1;
        }
        votes
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| format!("Speaker {}", n + 1))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// Text that may still change; replaces the previous partial.
    Partial(String),
    /// A finished line, `at_ms` after the call was admitted; `who` is its speaker when known.
    Final {
        at_ms: i64,
        text: String,
        who: Option<Who>,
    },
    /// Streaming failed; recording goes on.
    Unavailable,
    /// The owner or an admin paused recording, `at_ms` into the call.
    Paused { at_ms: i64 },
    /// Recording resumed. The span since the matching `Paused` is a gap.
    Resumed { at_ms: i64 },
    /// The call ended or someone opted out.
    End,
}

struct Call {
    started: Instant,
    lines: Vec<Line>,
    partial: String,
    unavailable: bool,
    /// Pauses so far as `(from, to)` in call ms; `to` is `None` while one is still open.
    gaps: Vec<(i64, Option<i64>)>,
    /// Someone with the right asked the notetaker to stop and leave.
    stop: bool,
    tx: broadcast::Sender<Msg>,
}

/// A finished line as kept: when, what and who.
pub type Line = (i64, String, Option<Who>);

/// What the call loop must do right now.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct Control {
    pub paused: bool,
    pub stop: bool,
}

/// What a viewer gets on connecting: the lines so far, then everything new.
pub struct Snapshot {
    pub lines: Vec<Line>,
    pub partial: String,
    pub unavailable: bool,
    pub gaps: Vec<(i64, Option<i64>)>,
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
                started: Instant::now(),
                lines: Vec::new(),
                partial: String::new(),
                unavailable: false,
                gaps: Vec::new(),
                stop: false,
                tx,
            },
        );
    }

    pub fn push(&self, id: &str, m: Msg) {
        let mut calls = self.calls.lock().unwrap();
        let Some(c) = calls.get_mut(id) else { return };
        match &m {
            Msg::Partial(t) => c.partial = t.clone(),
            Msg::Final { at_ms, text, who } => {
                c.partial.clear();
                c.lines.push((*at_ms, text.clone(), who.clone()));
                if c.lines.len() > MAX_LINES {
                    c.lines.remove(0);
                }
            }
            Msg::Unavailable => c.unavailable = true,
            Msg::Paused { .. } | Msg::Resumed { .. } | Msg::End => {}
        }
        let _ = c.tx.send(m);
    }

    /// Pauses or resumes recording; `None` when the call is not live here, and a repeat changes nothing.
    pub fn set_paused(&self, id: &str, paused: bool) -> Option<()> {
        let mut calls = self.calls.lock().unwrap();
        let c = calls.get_mut(id)?;
        let at_ms = c.started.elapsed().as_millis() as i64;
        let open = matches!(c.gaps.last(), Some((_, None)));
        if paused && !open {
            c.gaps.push((at_ms, None));
            let _ = c.tx.send(Msg::Paused { at_ms });
        } else if !paused && open {
            c.gaps.last_mut().unwrap().1 = Some(at_ms);
            let _ = c.tx.send(Msg::Resumed { at_ms });
        }
        Some(())
    }

    /// Asks the call loop to leave; `None` when the call is not live here.
    pub fn request_stop(&self, id: &str) -> Option<()> {
        self.calls.lock().unwrap().get_mut(id)?.stop = true;
        Some(())
    }

    /// What the call loop must do; nothing for a call that is not live here.
    pub fn control(&self, id: &str) -> Control {
        self.calls
            .lock()
            .unwrap()
            .get(id)
            .map(|c| Control {
                paused: matches!(c.gaps.last(), Some((_, None))),
                stop: c.stop,
            })
            .unwrap_or_default()
    }

    /// Drops the transcript and tells viewers the call is over.
    pub fn clear(&self, id: &str) {
        if let Some(c) = self.calls.lock().unwrap().remove(id) {
            let _ = c.tx.send(Msg::End);
        }
    }

    /// What was said in the last `window_ms` of the call, oldest first; `None` when the call is not live.
    // ponytail: the call clock is the time since `start`, which is when the call was admitted.
    pub fn recent(&self, id: &str, window_ms: i64) -> Option<Vec<String>> {
        let calls = self.calls.lock().unwrap();
        let c = calls.get(id)?;
        let from = c.started.elapsed().as_millis() as i64 - window_ms;
        Some(
            c.lines
                .iter()
                .filter(|(at, ..)| *at >= from)
                .map(|(_, t, _)| t.clone())
                .collect(),
        )
    }

    /// The finished line stamped `at_ms` with `n` lines either side; empty when there is no such line.
    pub fn around(&self, id: &str, at_ms: i64, n: usize) -> Option<Vec<String>> {
        let calls = self.calls.lock().unwrap();
        let c = calls.get(id)?;
        let Some(i) = c.lines.iter().position(|(at, ..)| *at == at_ms) else {
            return Some(Vec::new());
        };
        let (a, b) = (i.saturating_sub(n), (i + n + 1).min(c.lines.len()));
        Some(c.lines[a..b].iter().map(|(_, t, _)| t.clone()).collect())
    }

    /// Who has spoken so far and for how long (latest label per speaker number, ordered by number);
    /// empty when the call is not live here or nobody has spoken.
    pub fn speakers(&self, id: &str) -> Vec<Who> {
        let calls = self.calls.lock().unwrap();
        let mut by: std::collections::BTreeMap<u32, Who> = Default::default();
        for w in calls
            .get(id)
            .into_iter()
            .flat_map(|c| c.lines.iter().filter_map(|l| l.2.as_ref()))
        {
            let e = by.entry(w.n).or_insert_with(|| Who {
                n: w.n,
                label: String::new(),
                ms: 0,
            });
            e.label.clone_from(&w.label);
            e.ms += w.ms;
        }
        by.into_values().collect()
    }

    /// `None` when the call is not live here.
    pub fn subscribe(&self, id: &str) -> Option<Snapshot> {
        let calls = self.calls.lock().unwrap();
        let c = calls.get(id)?;
        Some(Snapshot {
            lines: c.lines.clone(),
            partial: c.partial.clone(),
            unavailable: c.unavailable,
            gaps: c.gaps.clone(),
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
    fn a_speaker_number_shows_its_most_active_name_or_speaker_n() {
        let mut l = Labels::default();
        // Browser and desktop recordings have no active-speaker samples: numbers, 1-based.
        assert_eq!(l.label(0, None), "Speaker 1");
        assert_eq!(l.label(2, None), "Speaker 3");
        // Samples name them: the name with the most votes wins, a tie goes to the first alphabetically.
        assert_eq!(l.label(0, Some("Ann")), "Ann");
        assert_eq!(l.label(0, Some("Bob")), "Ann");
        assert_eq!(l.label(0, Some("Bob")), "Bob");
        assert_eq!(l.label(2, None), "Speaker 3");
    }

    #[test]
    fn a_late_viewer_gets_the_finished_lines_and_the_current_partial() {
        let h = LiveHub::default();
        h.start("m");
        h.push(
            "m",
            Msg::Final {
                at_ms: 100,
                text: "hello".into(),
                who: Some(Who {
                    n: 0,
                    label: "Speaker 1".into(),
                    ms: 700,
                }),
            },
        );
        h.push("m", Msg::Partial("wor".into()));
        let s = h.subscribe("m").unwrap();
        let who = Who {
            n: 0,
            label: "Speaker 1".into(),
            ms: 700,
        };
        assert_eq!(s.lines, vec![(100, "hello".to_string(), Some(who))]);
        assert_eq!(s.partial, "wor");
        h.push(
            "m",
            Msg::Final {
                at_ms: 200,
                text: "world".into(),
                who: None,
            },
        );
        assert_eq!(h.subscribe("m").unwrap().partial, "");
    }

    #[test]
    fn pause_resume_and_stop_are_tracked_per_call_and_repeats_change_nothing() {
        let h = LiveHub::default();
        assert!(h.set_paused("m", true).is_none() && h.request_stop("m").is_none());
        assert_eq!(
            h.control("m"),
            Control::default(),
            "an unknown call has nothing to do"
        );
        h.start("m");
        let mut s = h.subscribe("m").unwrap();
        h.set_paused("m", true).unwrap();
        h.set_paused("m", true).unwrap();
        assert_eq!(
            h.control("m"),
            Control {
                paused: true,
                stop: false
            }
        );
        h.set_paused("m", false).unwrap();
        h.set_paused("m", false).unwrap();
        assert_eq!(h.control("m"), Control::default());
        h.request_stop("m").unwrap();
        assert_eq!(
            h.control("m"),
            Control {
                paused: false,
                stop: true
            }
        );
        let kinds: Vec<_> = std::iter::from_fn(|| s.rx.try_recv().ok())
            .map(|m| matches!(m, Msg::Paused { .. }))
            .collect();
        assert_eq!(
            kinds,
            [true, false],
            "one Paused then one Resumed, not one per repeat"
        );
        let gaps = h.subscribe("m").unwrap().gaps;
        assert_eq!((gaps.len(), gaps[0].1.is_some()), (1, true));
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
                who: None,
            },
        );
        h.clear("m");
        assert!(h.subscribe("m").is_none());
        assert_eq!(
            s.rx.try_recv().unwrap(),
            Msg::Final {
                at_ms: 1,
                text: "secret".into(),
                who: None
            }
        );
        assert_eq!(s.rx.try_recv().unwrap(), Msg::End);
        h.push(
            "m",
            Msg::Final {
                at_ms: 2,
                text: "late".into(),
                who: None,
            },
        );
        assert!(
            h.subscribe("m").is_none(),
            "a push after clear keeps nothing"
        );
    }

    #[test]
    fn recent_and_around_pick_lines_by_the_call_clock() {
        let h = LiveHub::default();
        assert!(h.recent("m", 60_000).is_none() && h.around("m", 0, 1).is_none());
        h.start("m");
        for (at, t) in [(0, "old"), (1, "a"), (2, "b"), (3, "c"), (4, "d")] {
            h.push(
                "m",
                Msg::Final {
                    at_ms: at,
                    text: t.into(),
                    who: None,
                },
            );
        }
        // The call is milliseconds old, so everything is inside 60 s and a 0 ms window holds nothing.
        assert_eq!(h.recent("m", 60_000).unwrap(), ["old", "a", "b", "c", "d"]);
        assert!(h.recent("m", -1_000_000).unwrap().is_empty());
        assert_eq!(h.around("m", 2, 1).unwrap(), ["a", "b", "c"]);
        assert_eq!(h.around("m", 0, 1).unwrap(), ["old", "a"]);
        assert!(h.around("m", 99, 1).unwrap().is_empty());
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
                    who: None,
                },
            );
        }
        let s = h.subscribe("m").unwrap();
        assert_eq!((s.lines.len(), s.lines[0].0), (MAX_LINES, 5));
    }
}
