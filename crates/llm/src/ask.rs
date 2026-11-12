//! Ask: answer a question from a few retrieved transcript excerpts and name the excerpts used.
//! Only excerpt text is sent to the provider, never a whole transcript.
use crate::{
    report::{generate, Generated},
    LlmError, Router, Task,
};
use serde::Deserialize;

pub const PROMPT_VERSION: u32 = 1;
pub const ASK: &str = include_str!("../prompts/ask.v1.txt");
/// Upper bound on excerpt text sent per question.
pub const MAX_CONTEXT_CHARS: usize = 12_000;
pub const NOT_FOUND: &str = "I could not find that in the meetings you can see.";

#[derive(Debug, Clone)]
pub struct Excerpt {
    pub title: String,
    pub start_ms: i64,
    pub text: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Reply {
    answer: String,
    #[serde(default)]
    citations: Vec<usize>,
}

#[derive(Debug, PartialEq)]
pub struct Answer {
    pub answer: String,
    /// Zero-based indexes into the excerpts passed in, in the order cited, without repeats.
    pub cited: Vec<usize>,
}

fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Numbers excerpts from 1 and stops before the character cap; returns the text and how many fit.
fn build(question: &str, excerpts: &[Excerpt]) -> (String, usize) {
    let mut body = String::new();
    let mut used = 0;
    for (i, e) in excerpts.iter().enumerate() {
        let line = format!(
            "[{}] ({}, {}) {}\n",
            i + 1,
            e.title,
            clock(e.start_ms),
            e.text
        );
        if used > 0 && body.chars().count() + line.chars().count() > MAX_CONTEXT_CHARS {
            break;
        }
        body.push_str(&line);
        used += 1;
    }
    (format!("{body}\nQuestion: {question}"), used)
}

pub async fn answer(
    router: &Router,
    question: &str,
    excerpts: &[Excerpt],
) -> Result<Generated<Answer>, LlmError> {
    if excerpts.is_empty() {
        return Ok(Generated {
            value: Answer {
                answer: NOT_FOUND.into(),
                cited: vec![],
            },
            calls: vec![],
        });
    }
    let (prompt, used) = build(question, excerpts);
    let g: Generated<Reply> = generate(router, Task::AskAnswer, ASK, &prompt).await?;
    let mut cited: Vec<usize> = vec![];
    for n in g.value.citations {
        // Numbers the model made up, or excerpts it was not shown, are dropped.
        if (1..=used).contains(&n) && !cited.contains(&(n - 1)) {
            cited.push(n - 1);
        }
    }
    Ok(Generated {
        value: Answer {
            answer: g.value.answer,
            cited,
        },
        calls: g.calls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{stub::Stub, Price, Route};
    use std::sync::Arc;

    fn router(stub: Arc<Stub>) -> Router {
        Router::new().provider("p", stub).route(
            Task::AskAnswer,
            Route {
                provider: "p".into(),
                model: "m".into(),
                max_tokens: 10,
                price: Price::default(),
            },
        )
    }

    fn ex(title: &str, ms: i64, text: &str) -> Excerpt {
        Excerpt {
            title: title.into(),
            start_ms: ms,
            text: text.into(),
        }
    }

    #[tokio::test]
    async fn cites_only_shown_excerpts_once_each_in_order() {
        let stub = Arc::new(Stub::texts(&[
            r#"{"answer":"Friday.","citations":[2,2,9,0,1]}"#,
        ]));
        let ex = [ex("Weekly", 65_000, "ships Friday"), ex("Sync", 0, "ok")];
        let g = answer(&router(stub.clone()), "when?", &ex).await.unwrap();
        assert_eq!(g.value.answer, "Friday.");
        assert_eq!(g.value.cited, vec![1, 0]);
        let sent = stub.requests();
        assert_eq!(sent[0].system, ASK);
        assert!(sent[0]
            .prompt
            .starts_with("[1] (Weekly, 01:05) ships Friday\n[2] (Sync, 00:00) ok\n"));
        assert!(sent[0].prompt.ends_with("Question: when?"));
    }

    #[tokio::test]
    async fn no_excerpts_means_no_model_call() {
        let stub = Arc::new(Stub::texts(&[]));
        let g = answer(&router(stub.clone()), "q", &[]).await.unwrap();
        assert_eq!(g.value.answer, NOT_FOUND);
        assert!(g.value.cited.is_empty() && g.calls.is_empty());
        assert!(stub.requests().is_empty());
    }

    #[tokio::test]
    async fn context_is_capped_and_late_excerpts_cannot_be_cited() {
        let big = "x".repeat(MAX_CONTEXT_CHARS - 100);
        let ex = [ex("A", 0, &big), ex("B", 0, &big)];
        let stub = Arc::new(Stub::texts(&[r#"{"answer":"a","citations":[1,2]}"#]));
        let g = answer(&router(stub.clone()), "q", &ex).await.unwrap();
        assert_eq!(g.value.cited, vec![0], "second excerpt was cut by the cap");
        assert!(!stub.requests()[0].prompt.contains("[2]"));
    }

    #[test]
    fn prompt_names_the_reply_keys() {
        for k in ["answer", "citations"] {
            assert!(ASK.contains(&format!("\"{k}\"")));
        }
    }
}
