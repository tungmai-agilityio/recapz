//! `report` job: build a transcript text, ask a model for the report, store it, mark the meeting ready.
//! The model sits behind `ReportModel`, so core needs no network code and tests need no provider.
use crate::{
    db::{new_id, now_ms},
    jobs::Job,
};
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};

pub const KIND: &str = "report";

pub struct ActionItemData {
    pub text: String,
    pub due: Option<String>,
    pub source_ms: Option<i64>,
}

/// One model call, shaped for an `llm_usage` row.
pub struct UsageData {
    pub task: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_micro_usd: u64,
}

pub struct ReportData {
    pub summary: String,
    pub decisions: Vec<String>,
    pub open_questions: Vec<String>,
    /// (start_ms, title)
    pub chapters: Vec<(i64, String)>,
    pub action_items: Vec<ActionItemData>,
    /// Model name and prompt version stored with the report.
    pub model: String,
    pub prompt_version: u32,
    pub usage: Vec<UsageData>,
}

pub trait ReportModel {
    /// `transcript` has one line per segment: `[mm:ss] Speaker: text`.
    fn generate(&self, transcript: &str) -> Result<ReportData>;
}

fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn transcript_text(c: &Connection, mid: &str) -> Result<String> {
    let mut q = c.prepare(
        "SELECT g.start_ms, COALESCE(p.display_name, p.label, 'Speaker'), g.text
         FROM segments g LEFT JOIN speakers p ON p.id = g.speaker_id
         WHERE g.meeting_id = ?1 ORDER BY g.start_ms, g.id",
    )?;
    let rows = q.query_map([mid], |r| {
        Ok(format!(
            "[{}] {}: {}",
            clock(r.get(0)?),
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?
        ))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?.join("\n"))
}

/// Safe to retry: the report and its action items are replaced, not appended.
pub fn handle(c: &Connection, job: &Job, model: &dyn ReportModel) -> Result<()> {
    let mid = job
        .meeting_id
        .as_deref()
        .ok_or_else(|| anyhow!("job has no meeting"))?;
    c.execute("UPDATE meetings SET status='reporting' WHERE id=?1", [mid])?;
    let transcript = transcript_text(c, mid)?;
    if transcript.is_empty() {
        anyhow::bail!("meeting has no transcript");
    }
    let data = model.generate(&transcript).context("report model")?;
    let now = now_ms();

    c.execute_batch("BEGIN")?;
    let stored = (|| -> Result<()> {
        c.execute(
            "INSERT OR REPLACE INTO reports
             (meeting_id, summary, decisions_json, questions_json, chapters_json, model, prompt_version, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                mid,
                data.summary,
                serde_json::to_string(&data.decisions)?,
                serde_json::to_string(&data.open_questions)?,
                serde_json::to_string(
                    &data.chapters.iter().map(|(ms, t)| serde_json::json!({"start_ms": ms, "title": t})).collect::<Vec<_>>()
                )?,
                data.model,
                data.prompt_version,
                now
            ],
        )?;
        c.execute("DELETE FROM action_items WHERE meeting_id=?1", [mid])?;
        for a in &data.action_items {
            c.execute(
                "INSERT INTO action_items (id, meeting_id, text, due_date, source_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![new_id(), mid, a.text, a.due, a.source_ms],
            )?;
        }
        for u in &data.usage {
            c.execute(
                "INSERT INTO llm_usage (meeting_id, task, provider, model, input_tokens, output_tokens, cost_micro_usd, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![mid, u.task, u.provider, u.model, u.input_tokens as i64, u.output_tokens as i64, u.cost_micro_usd as i64, now],
            )?;
        }
        c.execute(
            "UPDATE meetings SET status='ready', error=NULL WHERE id=?1",
            [mid],
        )?;
        Ok(())
    })();
    c.execute_batch(if stored.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    stored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::open_tenant, jobs};
    use std::cell::RefCell;

    struct Fake(RefCell<Vec<String>>);
    impl ReportModel for Fake {
        fn generate(&self, transcript: &str) -> Result<ReportData> {
            self.0.borrow_mut().push(transcript.to_string());
            Ok(ReportData {
                summary: "S".into(),
                decisions: vec!["d1".into()],
                open_questions: vec![],
                chapters: vec![(0, "Intro".into())],
                action_items: vec![ActionItemData {
                    text: "ship it".into(),
                    due: Some("2026-12-01".into()),
                    source_ms: Some(61_000),
                }],
                model: "m".into(),
                prompt_version: 1,
                usage: vec![UsageData {
                    task: "summary".into(),
                    provider: "p".into(),
                    model: "m".into(),
                    input_tokens: 10,
                    output_tokens: 5,
                    cost_micro_usd: 7,
                }],
            })
        }
    }

    struct Second;
    impl ReportModel for Second {
        fn generate(&self, _: &str) -> Result<ReportData> {
            Ok(ReportData {
                summary: "S2".into(),
                decisions: vec![],
                open_questions: vec![],
                chapters: vec![],
                action_items: vec![],
                model: "m".into(),
                prompt_version: 1,
                usage: vec![UsageData {
                    task: "summary".into(),
                    provider: "p".into(),
                    model: "m".into(),
                    input_tokens: 1,
                    output_tokens: 1,
                    cost_micro_usd: 1,
                }],
            })
        }
    }

    struct Broken;
    impl ReportModel for Broken {
        fn generate(&self, _: &str) -> Result<ReportData> {
            anyhow::bail!("provider down")
        }
    }

    fn setup() -> (tempfile::TempDir, Connection, Job) {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        c.execute_batch(
            "INSERT INTO meetings (id, title, platform, status) VALUES ('m1','t','upload','transcribing');
             INSERT INTO speakers (id, meeting_id, label, display_name) VALUES ('s1','m1','Speaker 1','Lan');
             INSERT INTO segments (meeting_id, speaker_id, start_ms, end_ms, text) VALUES
               ('m1','s1',61000,62000,'second'),
               ('m1',NULL,0,1000,'first');",
        )
        .unwrap();
        jobs::enqueue(&c, KIND, Some("m1"), None).unwrap();
        let job = jobs::claim(&c, &[KIND]).unwrap().unwrap();
        (dir, c, job)
    }

    fn count(c: &Connection, table: &str) -> i64 {
        c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn stores_report_items_usage_and_marks_ready_even_when_rerun() {
        let (_d, c, job) = setup();
        let fake = Fake(RefCell::new(vec![]));
        for _ in 0..2 {
            handle(&c, &job, &fake).unwrap();
        }
        assert_eq!(
            fake.0.borrow()[0],
            "[00:00] Speaker: first\n[01:01] Lan: second"
        );
        let (summary, chapters, v): (String, String, i64) = c
            .query_row(
                "SELECT summary, chapters_json, prompt_version FROM reports WHERE meeting_id='m1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (summary.as_str(), chapters.as_str(), v),
            ("S", r#"[{"start_ms":0,"title":"Intro"}]"#, 1)
        );
        assert_eq!(count(&c, "reports"), 1);
        assert_eq!(
            count(&c, "action_items"),
            1,
            "rerun must not duplicate items"
        );
        let (due, src): (String, i64) = c
            .query_row("SELECT due_date, source_ms FROM action_items", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((due.as_str(), src), ("2026-12-01", 61_000));
        assert_eq!(count(&c, "llm_usage"), 2, "one usage row per call per run");
        let st: String = c
            .query_row("SELECT status FROM meetings WHERE id='m1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(st, "ready");
    }

    #[test]
    fn model_failure_stores_nothing_and_keeps_reporting_status() {
        let (_d, c, job) = setup();
        assert!(handle(&c, &job, &Broken).is_err());
        assert_eq!(
            count(&c, "reports") + count(&c, "action_items") + count(&c, "llm_usage"),
            0
        );
        let st: String = c
            .query_row("SELECT status FROM meetings WHERE id='m1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(st, "reporting");
    }

    #[test]
    fn write_failure_rolls_back_to_the_previous_report() {
        let (_d, c, job) = setup();
        handle(&c, &job, &Fake(RefCell::new(vec![]))).unwrap();
        // The usage insert is the last write, after the report and items were replaced.
        c.execute("DROP TABLE llm_usage", []).unwrap();
        assert!(handle(&c, &job, &Second).is_err());
        let summary: String = c
            .query_row(
                "SELECT summary FROM reports WHERE meeting_id='m1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(summary, "S", "old report must survive");
        assert_eq!(count(&c, "action_items"), 1, "old items must survive");
        let st: String = c
            .query_row("SELECT status FROM meetings WHERE id='m1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(st, "reporting");
    }

    #[test]
    fn empty_transcript_is_an_error() {
        let (_d, c, job) = setup();
        c.execute("DELETE FROM segments", []).unwrap();
        let fake = Fake(RefCell::new(vec![]));
        assert!(handle(&c, &job, &fake).is_err());
        assert!(
            fake.0.borrow().is_empty(),
            "model must not be called without a transcript"
        );
    }
}
