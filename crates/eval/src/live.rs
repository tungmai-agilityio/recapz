//! Live answer accuracy: ingest synthetic client documents the way the server does, retrieve for
//! each question, ask the model through `live_answer.v1` and score the reply. A question with a
//! `doc` is correct when the reply is found, cites that document and states the expected fact. A
//! question with `doc: null` is correct only when the reply says it is not in the documents and
//! cites nothing. Sends only the synthetic fixture text to the provider.
use crate::ask::{AskReport, TOP_K};
use anyhow::{anyhow, Context, Result};
use recapz_core::{db::open_tenant, document, embed::HashEmbedder, jobs::Job, search::search};
use recapz_llm::ask::{live_answer, Excerpt, LiveAnswer, Source};
use recapz_llm::Router;
use serde_json::Value;

/// What a question must get back.
#[derive(Debug, PartialEq)]
pub struct Expect<'a> {
    /// Name of the document that holds the answer; `None` when no document does.
    pub doc: Option<&'a str>,
    /// A fact the answer must state (compared after `normalize`).
    pub must: &'a str,
}

/// `titles[i]` is the document name of excerpt `i`.
pub fn judge(e: &Expect, a: &LiveAnswer, titles: &[String]) -> bool {
    match e.doc {
        None => !a.found && a.cited.is_empty(),
        Some(doc) => {
            a.found
                && !a.cited.is_empty()
                && a.cited
                    .iter()
                    .all(|&i| titles.get(i).map(String::as_str) == Some(doc))
                && crate::normalize(&a.answer).contains(&crate::normalize(e.must))
        }
    }
}

fn items<'a>(set: &'a Value, k: &str) -> Result<&'a Vec<Value>> {
    set.get(k)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("'{k}' must be a list"))
}

fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("'{k}' must be text"))
}

/// Excerpts (and their document names) retrieval gives for each question.
type Retrieved = Vec<(Vec<Excerpt>, Vec<String>)>;

fn retrieve(dir: &std::path::Path, set: &Value) -> Result<Retrieved> {
    let c = open_tenant(dir, "eval")?;
    c.execute(
        "INSERT INTO members (id, email, role) VALUES ('eval', 'eval@local', 'admin')",
        [],
    )?;
    for d in items(set, "documents")? {
        let id = document::add(
            &c,
            dir,
            "eval",
            "eval",
            text(d, "name")?,
            text(d, "text")?.as_bytes(),
        )?;
        document::set_llm_ok(&c, "eval", &id, true)?;
        let job = Job {
            id: 0,
            kind: document::KIND.into(),
            meeting_id: None,
            payload: Some(id),
        };
        document::handle(&c, dir, "eval", &job, &HashEmbedder).context("ingest")?;
    }
    items(set, "questions")?
        .iter()
        .map(|q| {
            let hits = search(&c, "eval", text(q, "q")?, &HashEmbedder, TOP_K)?;
            let titles = hits.iter().map(|h| h.title.clone()).collect();
            let ex = hits
                .into_iter()
                .map(|h| Excerpt {
                    source: Source::Document { title: h.title },
                    text: h.text,
                })
                .collect();
            Ok((ex, titles))
        })
        .collect()
}

/// `set` is `{"documents":[{name,text}],"questions":[{q,doc,must}]}`; `doc` is null when no document
/// holds the answer.
pub async fn run(set: &Value, router: &Router) -> Result<AskReport> {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("recapz-eval-live-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let retrieved = retrieve(&dir, set);
    let _ = std::fs::remove_dir_all(&dir);
    let mut r = AskReport {
        total: 0,
        correct: 0,
        misses: vec![],
    };
    for (q, (ex, titles)) in items(set, "questions")?.iter().zip(retrieved?) {
        let question = text(q, "q")?;
        let expect = Expect {
            doc: q.get("doc").and_then(Value::as_str),
            must: q.get("must").and_then(Value::as_str).unwrap_or(""),
        };
        let a = live_answer(router, question, &ex).await?.value;
        r.total += 1;
        if judge(&expect, &a, &titles) {
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
    use crate::{ask::PASS_RATE, normalize};
    use async_trait::async_trait;
    use recapz_llm::{Completion, CompletionRequest, LlmError, Price, Provider, Route, Task};
    use std::sync::Arc;

    fn set() -> Value {
        serde_json::from_str(include_str!("../fixtures/live-answer-set.json")).unwrap()
    }

    /// A model that reads the excerpts: finds the one that states the expected fact and cites it.
    struct Oracle(Value);

    #[async_trait]
    impl Provider for Oracle {
        async fn complete(&self, req: CompletionRequest) -> Result<Completion, LlmError> {
            let q = req.prompt.rsplit("Question: ").next().unwrap();
            let want = items(&self.0, "questions")
                .unwrap()
                .iter()
                .find(|x| x["q"] == q)
                .unwrap();
            let must = want["must"].as_str().unwrap_or("");
            let body = req.prompt.split("\n\nQuestion: ").next().unwrap();
            let hit = body
                .split("\n[")
                .find(|e| want["doc"].is_string() && normalize(e).contains(&normalize(must)));
            let text = match hit {
                Some(e) => {
                    let n: String = e
                        .trim_start_matches('[')
                        .chars()
                        .take_while(char::is_ascii_digit)
                        .collect();
                    format!(r#"{{"found":true,"answer":"{must}","citations":[{n}]}}"#)
                }
                None => {
                    r#"{"found":false,"answer":"Not in your documents.","citations":[]}"#.into()
                }
            };
            Ok(Completion {
                text,
                input_tokens: 1,
                output_tokens: 1,
            })
        }
    }

    fn router(p: impl Provider + 'static) -> Router {
        Router::new().provider("p", Arc::new(p)).route(
            Task::LiveAnswer,
            Route {
                provider: "p".into(),
                model: "m".into(),
                max_tokens: 10,
                price: Price::default(),
            },
        )
    }

    struct Fixed(&'static str);

    #[async_trait]
    impl Provider for Fixed {
        async fn complete(&self, _: CompletionRequest) -> Result<Completion, LlmError> {
            Ok(Completion {
                text: self.0.into(),
                input_tokens: 1,
                output_tokens: 1,
            })
        }
    }

    #[test]
    fn fixture_is_30_questions_and_every_expected_fact_is_in_its_document() {
        let s = set();
        let qs = items(&s, "questions").unwrap();
        assert_eq!(qs.len(), 30);
        assert_eq!(qs.iter().filter(|q| q["doc"].is_null()).count(), 6);
        for q in qs.iter().filter(|q| q["doc"].is_string()) {
            let d = items(&s, "documents")
                .unwrap()
                .iter()
                .find(|d| d["name"] == q["doc"])
                .unwrap_or_else(|| panic!("no document for {}", q["q"]));
            assert!(
                normalize(d["text"].as_str().unwrap())
                    .contains(&normalize(q["must"].as_str().unwrap())),
                "{}",
                q["q"]
            );
        }
    }

    #[tokio::test]
    async fn a_model_that_reads_the_excerpts_scores_every_question() {
        let s = set();
        let r = run(&s, &router(Oracle(s.clone()))).await.unwrap();
        assert_eq!((r.total, r.correct), (30, 30), "{:?}", r.misses);
        assert!(r.accuracy() >= PASS_RATE);
    }

    #[tokio::test]
    async fn always_not_found_gets_only_the_not_in_documents_questions_right() {
        let r = run(
            &set(),
            &router(Fixed(r#"{"found":false,"answer":"No.","citations":[]}"#)),
        )
        .await
        .unwrap();
        assert_eq!((r.total, r.correct), (30, 6));
        assert!(r.accuracy() < PASS_RATE);
    }

    #[tokio::test]
    async fn always_found_with_an_empty_answer_gets_nothing_right() {
        let r = run(
            &set(),
            &router(Fixed(r#"{"found":true,"answer":"Yes.","citations":[1]}"#)),
        )
        .await
        .unwrap();
        assert_eq!((r.total, r.correct), (30, 0));
    }

    fn a(found: bool, answer: &str, cited: &[usize]) -> LiveAnswer {
        LiveAnswer {
            found,
            answer: answer.into(),
            cited: cited.to_vec(),
        }
    }

    #[test]
    fn judge_needs_the_right_document_the_fact_and_a_citation() {
        let t = ["a.md".to_string(), "b.md".to_string()];
        let e = Expect {
            doc: Some("b.md"),
            must: "net 30",
        };
        assert!(judge(&e, &a(true, "Terms are Net 30.", &[1]), &t));
        assert!(
            !judge(&e, &a(true, "Terms are Net 30.", &[0]), &t),
            "wrong document"
        );
        assert!(
            !judge(&e, &a(true, "Terms are Net 30.", &[0, 1]), &t),
            "also cites another document"
        );
        assert!(
            !judge(&e, &a(true, "Terms are Net 30.", &[]), &t),
            "no citation"
        );
        assert!(
            !judge(&e, &a(true, "Terms are net 45.", &[1]), &t),
            "wrong fact"
        );
        assert!(
            !judge(&e, &a(false, "Terms are net 30.", &[1]), &t),
            "said not found"
        );
        let none = Expect {
            doc: None,
            must: "",
        };
        assert!(judge(&none, &a(false, "Not there.", &[]), &t));
        assert!(
            !judge(&none, &a(true, "Net 30.", &[0]), &t),
            "made something up"
        );
    }

    #[tokio::test]
    async fn a_malformed_set_is_an_error() {
        let r = router(Fixed("{}"));
        assert!(run(&serde_json::json!({"documents": 1}), &r).await.is_err());
    }
}
