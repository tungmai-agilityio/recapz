//! Ask retrieval accuracy: index a set of transcripts the way the server does, run each question
//! through hybrid search and check the right meeting comes back at about the right moment.
//! Measures retrieval only (no model call), so it runs offline and costs nothing.
use anyhow::{anyhow, Context, Result};
use recapz_core::{db::open_tenant, embed::HashEmbedder, index_job, jobs::Job, search::search};
use rusqlite::params;
use serde_json::Value;

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

/// `set` is `{"meetings":[{id,title,segments:[{start_ms,end_ms,text}]}],"questions":[{q,meeting,at_ms}]}`.
pub fn run(set: &Value) -> Result<AskReport> {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("recapz-eval-ask-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let report = run_in(&dir, set);
    let _ = std::fs::remove_dir_all(&dir);
    report
}

fn run_in(dir: &std::path::Path, set: &Value) -> Result<AskReport> {
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
        index_job::handle(&c, &job, &HashEmbedder).context("index")?;
    }
    let mut r = AskReport {
        total: 0,
        correct: 0,
        misses: vec![],
    };
    for q in list(set, "questions")? {
        let (question, meeting, at) = (text(q, "q")?, text(q, "meeting")?, num(q, "at_ms")?);
        let hits = search(&c, "eval", question, &HashEmbedder, TOP_K)?;
        let ok = hits.iter().any(|h| {
            h.meeting_id == meeting && h.start_ms - SLACK_MS <= at && at <= h.end_ms + SLACK_MS
        });
        r.total += 1;
        if ok {
            r.correct += 1;
        } else {
            r.misses.push(question.to_string());
        }
    }
    Ok(r)
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

    #[test]
    fn malformed_set_is_an_error() {
        assert!(run(&serde_json::json!({"meetings": 1})).is_err());
    }
}
