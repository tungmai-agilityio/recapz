//! Report prompt quality: run the four report tasks on synthetic transcripts and check the replies
//! parse, look sane, and cover the decisions, open questions and action items planted in the set.
//! Sends only the synthetic fixture text to the provider.
use crate::{ask::AskReport, normalize};
use anyhow::{anyhow, Result};
use recapz_llm::report::{
    generate, ActionItems, Chapters, Decisions, Summary, ACTION_ITEMS, CHAPTERS, DECISIONS, SUMMARY,
};
use recapz_llm::{LlmError, Router, Task};
use serde_json::Value;

fn items<'a>(v: &'a Value, k: &str) -> Result<&'a Vec<Value>> {
    v.get(k)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("'{k}' must be a list"))
}

fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("'{k}' must be text"))
}

/// Milliseconds of the last `[mm:ss]` line.
fn last_ms(transcript: &str) -> i64 {
    transcript
        .lines()
        .filter_map(|l| {
            let (m, s) = l.strip_prefix('[')?.split_once(']')?.0.split_once(':')?;
            Some((m.parse::<i64>().ok()? * 60 + s.parse::<i64>().ok()?) * 1000)
        })
        .max()
        .unwrap_or(0)
}

fn iso_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

fn covers(haystack: &[&str], needle: &str) -> bool {
    let n = normalize(needle);
    haystack.iter().any(|h| normalize(h).contains(&n))
}

/// Checks per meeting: summary, chapters and due dates are well formed (3), then one check per
/// planted decision, open question and action item (owner and a keyword in one item).
async fn score(m: &Value, router: &Router, r: &mut AskReport) -> Result<(), LlmError> {
    let t = m["transcript"].as_str().unwrap_or("");
    let name = m["name"].as_str().unwrap_or("?");
    let s = generate::<Summary>(router, Task::Summary, SUMMARY, t)
        .await?
        .value;
    let d = generate::<Decisions>(router, Task::Decisions, DECISIONS, t)
        .await?
        .value;
    let c = generate::<Chapters>(router, Task::Chapters, CHAPTERS, t)
        .await?
        .value;
    let a = generate::<ActionItems>(router, Task::ActionItems, ACTION_ITEMS, t)
        .await?
        .value;
    let mut check = |ok: bool, what: String| {
        r.total += 1;
        if ok {
            r.correct += 1;
        } else {
            r.misses.push(format!("{name}: {what}"));
        }
    };
    check(!s.summary.trim().is_empty(), "empty summary".into());
    let ms: Vec<i64> = c.chapters.iter().map(|c| c.start_ms).collect();
    check(
        !ms.is_empty()
            && ms.windows(2).all(|w| w[0] < w[1])
            && ms.iter().all(|&x| (0..=last_ms(t)).contains(&x)),
        format!("chapter times not ascending within the meeting: {ms:?}"),
    );
    check(
        a.action_items
            .iter()
            .all(|i| i.due.as_deref().is_none_or(iso_date)),
        "action item due is not an ISO date".into(),
    );
    let decisions: Vec<&str> = d.decisions.iter().map(String::as_str).collect();
    let questions: Vec<&str> = d.open_questions.iter().map(String::as_str).collect();
    for k in items(m, "decisions").unwrap_or(&vec![]) {
        let k = k.as_str().unwrap_or("");
        check(covers(&decisions, k), format!("decision missing: {k}"));
    }
    for k in items(m, "open_questions").unwrap_or(&vec![]) {
        let k = k.as_str().unwrap_or("");
        check(covers(&questions, k), format!("open question missing: {k}"));
    }
    for p in items(m, "action_items").unwrap_or(&vec![]) {
        let (owner, must) = (
            p["owner"].as_str().unwrap_or(""),
            p["must"].as_str().unwrap_or(""),
        );
        let found = a.action_items.iter().any(|i| {
            covers(&[&i.text], must)
                && i.owner
                    .as_deref()
                    .is_some_and(|o| normalize(o) == normalize(owner))
        });
        check(found, format!("action item missing: {owner} / {must}"));
    }
    Ok(())
}

/// `set` is `{"meetings":[{name,transcript,decisions,open_questions,action_items:[{owner,must}]}]}`.
/// A meeting whose model calls fail counts as one miss.
pub async fn run(set: &Value, router: &Router) -> Result<AskReport> {
    let mut r = AskReport {
        total: 0,
        correct: 0,
        misses: vec![],
    };
    for m in items(set, "meetings")? {
        text(m, "transcript")?;
        if let Err(e) = score(m, router, &mut r).await {
            r.total += 1;
            r.misses.push(format!(
                "{}: model call failed ({e})",
                m["name"].as_str().unwrap_or("?")
            ));
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask::PASS_RATE;
    use async_trait::async_trait;
    use recapz_llm::{Completion, CompletionRequest, Price, Provider, Route};
    use std::sync::Arc;

    fn set() -> Value {
        serde_json::from_str(include_str!("../fixtures/report-set.json")).unwrap()
    }

    /// Replies per task from `by_task`, picked by the key the system prompt asks for.
    struct Canned(fn(&str) -> String);

    #[async_trait]
    impl Provider for Canned {
        async fn complete(&self, req: CompletionRequest) -> Result<Completion, LlmError> {
            let key = ["action_items", "chapters", "open_questions", "summary"]
                .into_iter()
                .find(|k| req.system.contains(&format!("\"{k}\"")))
                .unwrap();
            Ok(Completion {
                text: (self.0)(key),
                input_tokens: 1,
                output_tokens: 1,
            })
        }
    }

    fn router(f: fn(&str) -> String) -> Router {
        let route = |_| Route {
            provider: "p".into(),
            model: "m".into(),
            max_tokens: 10,
            price: Price::default(),
        };
        Router::new()
            .provider("p", Arc::new(Canned(f)))
            .route(Task::Summary, route(0))
            .route(Task::Decisions, route(0))
            .route(Task::Chapters, route(0))
            .route(Task::ActionItems, route(0))
    }

    /// Knows both fixture meetings (an English and a Vietnamese one): answers with everything planted.
    fn perfect(key: &str) -> String {
        match key {
            "summary" => r#"{"summary":"A meeting."}"#,
            "chapters" => r#"{"chapters":[{"start_ms":0,"title":"a"},{"start_ms":60000,"title":"b"}]}"#,
            "open_questions" => r#"{"decisions":["Ship on 12 May","Start with a 14 day trial","Mở kho Đà Nẵng","Giữ 50 triệu mỗi tháng"],"open_questions":["Separate privacy notice?","Nhận ví điện tử?"]}"#,
            _ => r#"{"action_items":[
                {"text":"Send the final screenshots to Priya","owner":"Jonas","due":null,"source_ms":90000},
                {"text":"Brief the support team on Monday","owner":"Maya","due":null,"source_ms":210000},
                {"text":"Gửi bảng so sánh ba địa điểm kho","owner":"Hùng","due":"2026-12-05","source_ms":130000},
                {"text":"Làm lại trang chính sách đổi trả","owner":"Mai","due":null,"source_ms":160000}]}"#,
        }
        .into()
    }

    #[test]
    fn fixture_plants_are_in_the_transcripts() {
        for m in items(&set(), "meetings").unwrap() {
            let t = normalize(m["transcript"].as_str().unwrap());
            for p in m["action_items"].as_array().unwrap() {
                assert!(t.contains(&normalize(p["must"].as_str().unwrap())), "{p}");
                assert!(t.contains(&normalize(p["owner"].as_str().unwrap())), "{p}");
            }
        }
    }

    #[tokio::test]
    async fn a_report_with_everything_planted_scores_every_check() {
        let r = run(&set(), &router(perfect)).await.unwrap();
        // 2 meetings x (3 shape checks + 2 decisions + 1 question + 2 action items)
        assert_eq!((r.total, r.correct), (16, 16), "{:?}", r.misses);
        assert!(r.accuracy() >= PASS_RATE);
    }

    #[tokio::test]
    async fn an_empty_report_fails_coverage_and_shape() {
        fn empty(key: &str) -> String {
            match key {
                "summary" => r#"{"summary":" "}"#,
                "chapters" => r#"{"chapters":[{"start_ms":99999999,"title":"x"}]}"#,
                "open_questions" => r#"{"decisions":[],"open_questions":[]}"#,
                _ => r#"{"action_items":[{"text":"x","owner":null,"due":"Friday","source_ms":0}]}"#,
            }
            .into()
        }
        let r = run(&set(), &router(empty)).await.unwrap();
        assert_eq!((r.total, r.correct), (16, 0));
        assert!(r.accuracy() < PASS_RATE);
        assert!(r
            .misses
            .contains(&"en-launch-planning: decision missing: 12 may".to_string()));
    }

    #[tokio::test]
    async fn a_model_failure_is_a_miss_not_an_abort() {
        let r = run(&set(), &Router::new()).await.unwrap();
        assert_eq!((r.total, r.correct, r.misses.len()), (2, 0, 2));
    }

    #[test]
    fn last_ms_reads_the_final_timestamp() {
        assert_eq!(last_ms("[00:05] A: x\n[02:30] B: y\n"), 150_000);
        assert!(iso_date("2026-12-05") && !iso_date("Friday"));
    }
}
