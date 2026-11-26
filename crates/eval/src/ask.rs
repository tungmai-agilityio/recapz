//! Ask retrieval accuracy: index a set of transcripts the way the server does, run each question
//! through hybrid search and check the right meeting comes back at about the right moment.
//! Measures retrieval only (no model call), so it runs offline and costs nothing.
use anyhow::{anyhow, Context, Result};
use recapz_core::{
    db::open_tenant,
    embed::{Embedder, HashEmbedder},
    index_job,
    jobs::Job,
    search::search_at,
};
use rusqlite::params;
use serde_json::Value;
use std::{collections::HashMap, sync::Mutex};

/// A hit counts when its chunk comes within this many ms of the expected moment.
pub const SLACK_MS: i64 = 30_000;
/// Chunks examined per question (what Ask would show the model).
pub const TOP_K: usize = 3;
pub const PASS_RATE: f64 = 0.8;

#[derive(Debug)]
pub struct AskReport {
    pub total: usize,
    pub correct: usize,
    /// Question text of every miss.
    pub misses: Vec<String>,
}

impl AskReport {
    pub fn accuracy(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.correct as f64 / self.total as f64
        }
    }
}

fn field<'a>(v: &'a Value, k: &str) -> Result<&'a Value> {
    v.get(k).ok_or_else(|| anyhow!("missing '{k}'"))
}

fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    field(v, k)?
        .as_str()
        .ok_or_else(|| anyhow!("'{k}' must be text"))
}

fn num(v: &Value, k: &str) -> Result<i64> {
    field(v, k)?
        .as_i64()
        .ok_or_else(|| anyhow!("'{k}' must be a number"))
}

fn list<'a>(v: &'a Value, k: &str) -> Result<&'a Vec<Value>> {
    field(v, k)?
        .as_array()
        .ok_or_else(|| anyhow!("'{k}' must be a list"))
}

/// Cosine cut-offs the sweep tries: 0.0 to 0.6 in steps of 0.05.
pub fn thresholds() -> Vec<f32> {
    (0..=12).map(|i| i as f32 * 0.05).collect()
}

/// Pass rate at each threshold, plus the rate at the embedder's own `min_similarity`.
pub struct Sweep {
    pub rates: Vec<(f32, AskReport)>,
    pub current: (f32, AskReport),
}

/// Remembers each text's vector, so a sweep asks a hosted embedder once per text, not once per threshold.
struct Memo<'a> {
    inner: &'a dyn Embedder,
    seen: Mutex<HashMap<String, Vec<f32>>>,
}

impl Embedder for Memo<'_> {
    fn id(&self) -> &'static str {
        self.inner.id()
    }
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if let Some(v) = self.seen.lock().unwrap().get(text) {
            return Ok(v.clone());
        }
        let v = self.inner.embed(text)?;
        self.seen.lock().unwrap().insert(text.into(), v.clone());
        Ok(v)
    }
    fn min_similarity(&self) -> f32 {
        self.inner.min_similarity()
    }
}

/// `set` is `{"meetings":[{id,title,segments:[{start_ms,end_ms,text}]}],"questions":[{q,meeting,at_ms}]}`.
/// Scores at the built-in baseline embedder's own threshold.
pub fn run(set: &Value) -> Result<AskReport> {
    Ok(sweep(set, &HashEmbedder)?.current.1)
}

/// Indexes the set with `embedder`, then scores every question at each threshold and at the
/// embedder's own. A hosted embedder sends the set's text to its provider (synthetic text only).
pub fn sweep(set: &Value, embedder: &dyn Embedder) -> Result<Sweep> {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("recapz-eval-ask-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let memo = Memo {
        inner: embedder,
        seen: Mutex::default(),
    };
    let report = sweep_in(&dir, set, &memo);
    let _ = std::fs::remove_dir_all(&dir);
    report
}

fn sweep_in(dir: &std::path::Path, set: &Value, e: &dyn Embedder) -> Result<Sweep> {
    let c = open_tenant(dir, "eval")?;
    c.execute(
        "INSERT INTO members (id, email, role) VALUES ('eval', 'eval@local', 'admin')",
        [],
    )?;
    for m in list(set, "meetings")? {
        let id = text(m, "id")?;
        c.execute(
            "INSERT INTO meetings (id, title, platform, status, owner_id) VALUES (?1, ?2, 'upload', 'ready', 'eval')",
            params![id, text(m, "title")?],
        )?;
        for s in list(m, "segments")? {
            c.execute(
                "INSERT INTO segments (meeting_id, start_ms, end_ms, text) VALUES (?1, ?2, ?3, ?4)",
                params![id, num(s, "start_ms")?, num(s, "end_ms")?, text(s, "text")?],
            )?;
        }
        let job = Job {
            id: 0,
            kind: index_job::KIND.into(),
            meeting_id: Some(id.into()),
            payload: None,
        };
        index_job::handle(&c, &job, e).context("index")?;
    }
    let score = |min: f32| -> Result<AskReport> {
        let mut r = AskReport {
            total: 0,
            correct: 0,
            misses: vec![],
        };
        for q in list(set, "questions")? {
            let (question, meeting, at) = (text(q, "q")?, text(q, "meeting")?, num(q, "at_ms")?);
            let hits = search_at(&c, "eval", question, e, TOP_K, min)?;
            let ok = hits.iter().any(|h| {
                h.meeting_id.as_deref() == Some(meeting)
                    && h.start_ms - SLACK_MS <= at
                    && at <= h.end_ms + SLACK_MS
            });
            r.total += 1;
            if ok {
                r.correct += 1;
            } else {
                r.misses.push(question.to_string());
            }
        }
        Ok(r)
    };
    let rates = thresholds()
        .into_iter()
        .map(|t| Ok((t, score(t)?)))
        .collect::<Result<_>>()?;
    Ok(Sweep {
        rates,
        current: (e.min_similarity(), score(e.min_similarity())?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        serde_json::from_str(include_str!("../fixtures/ask-sample.json")).unwrap()
    }

    #[test]
    fn sample_set_meets_the_pass_rate_with_the_baseline_embedder() {
        let r = run(&sample()).unwrap();
        assert_eq!(r.total, 12);
        assert!(r.accuracy() >= PASS_RATE, "{:?}", r.misses);
    }

    #[test]
    fn a_wrong_expected_moment_or_meeting_is_a_miss() {
        let mut s = sample();
        // Right words, but a moment 100 s later than anything said about it.
        s["questions"][0]["at_ms"] = Value::from(110_000);
        // Right moment, wrong meeting.
        s["questions"][1]["meeting"] = Value::from("m2");
        let r = run(&s).unwrap();
        assert_eq!(r.total, 12);
        assert!(r
            .misses
            .contains(&"When does the pricing page ship?".to_string()));
        assert!(r
            .misses
            .contains(&"What discount did marketing want for the annual plan?".to_string()));
        assert!(r.correct <= 10);
    }

    /// Text with "alpha" gets (1, 0); any other text gets a vector whose cosine with it is 0.42.
    struct Tilt;
    impl Embedder for Tilt {
        fn id(&self) -> &'static str {
            "tilt"
        }
        fn embed(&self, t: &str) -> Result<Vec<f32>> {
            Ok(if t.contains("alpha") {
                vec![1.0, 0.0]
            } else {
                vec![0.42, 0.90752]
            })
        }
    }

    #[test]
    fn the_sweep_shows_where_a_vector_only_match_stops_counting() {
        let set = serde_json::json!({
            "meetings": [{"id": "m", "title": "T", "segments": [{"start_ms": 0, "end_ms": 5000, "text": "alpha beta"}]}],
            "questions": [{"q": "zzz", "meeting": "m", "at_ms": 1000}]
        });
        let s = sweep(&set, &Tilt).unwrap();
        let hits: Vec<usize> = s.rates.iter().map(|(_, r)| r.correct).collect();
        // 0.00 to 0.40 find it (cosine 0.42), 0.45 to 0.60 do not.
        assert_eq!(hits, [1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0]);
        assert_eq!(s.rates.len(), thresholds().len());
        assert_eq!((s.current.0, s.current.1.correct), (0.2, 1));
    }

    #[test]
    fn malformed_set_is_an_error() {
        assert!(run(&serde_json::json!({"meetings": 1})).is_err());
    }
}
