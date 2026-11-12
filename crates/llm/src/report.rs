//! Meeting report tasks: versioned prompts, JSON replies validated by shape, one retry.
use crate::{LlmError, Routed, Router, Task};
use serde::de::DeserializeOwned;
use serde::Deserialize;

pub const PROMPT_VERSION: u32 = 1;

pub const SUMMARY: &str = include_str!("../prompts/summary.v1.txt");
pub const DECISIONS: &str = include_str!("../prompts/decisions.v1.txt");
pub const CHAPTERS: &str = include_str!("../prompts/chapters.v1.txt");
pub const ACTION_ITEMS: &str = include_str!("../prompts/action_items.v1.txt");

#[derive(Debug, Deserialize, PartialEq)]
pub struct Summary {
    pub summary: String,
}
#[derive(Debug, Deserialize, PartialEq)]
pub struct Decisions {
    pub decisions: Vec<String>,
    pub open_questions: Vec<String>,
}
#[derive(Debug, Deserialize, PartialEq)]
pub struct Chapter {
    pub start_ms: i64,
    pub title: String,
}
#[derive(Debug, Deserialize, PartialEq)]
pub struct Chapters {
    pub chapters: Vec<Chapter>,
}
#[derive(Debug, Deserialize, PartialEq)]
pub struct ActionItem {
    pub text: String,
    pub owner: Option<String>,
    pub due: Option<String>,
    pub source_ms: Option<i64>,
}
#[derive(Debug, Deserialize, PartialEq)]
pub struct ActionItems {
    pub action_items: Vec<ActionItem>,
}

/// A parsed value plus every call made for it (one `llm_usage` row each).
pub struct Generated<T> {
    pub value: T,
    pub calls: Vec<Routed>,
}

/// Models often wrap JSON in a code fence; take the outermost `{...}`.
fn json_slice(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(a), Some(b)) if a < b => &text[a..=b],
        _ => text,
    }
}

/// Run a task and parse its reply. On a bad reply, retry once with the error; then fail.
pub async fn generate<T: DeserializeOwned>(
    router: &Router,
    task: Task,
    system: &str,
    transcript: &str,
) -> Result<Generated<T>, LlmError> {
    let mut calls = Vec::new();
    let mut prompt = transcript.to_string();
    let mut category = String::new();
    for _ in 0..2 {
        let out = router.run(task, system, &prompt).await?;
        let parsed = serde_json::from_str::<T>(json_slice(&out.completion.text));
        let text = out.completion.text.clone();
        calls.push(out);
        match parsed {
            Ok(value) => return Ok(Generated { value, calls }),
            Err(e) => {
                // Full text goes back to the provider only; the returned error keeps the category so
                // transcript text never reaches logs.
                category = format!("{:?}", e.classify());
                let last = e.to_string();
                prompt = format!(
                    "{transcript}\n\nYour previous reply was not valid: {last}. Reply again with the JSON only.\nPrevious reply:\n{text}"
                );
            }
        }
    }
    Err(LlmError::Provider(format!(
        "invalid JSON after retry ({category})"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stub::Stub;
    use crate::{Price, Route};
    use std::sync::Arc;

    fn router(stub: Arc<Stub>) -> Router {
        Router::new().provider("p", stub).route(
            Task::Summary,
            Route {
                provider: "p".into(),
                model: "m".into(),
                max_tokens: 10,
                price: Price::default(),
            },
        )
    }

    #[tokio::test]
    async fn parses_fenced_json() {
        let stub = Arc::new(Stub::texts(&["```json\n{\"summary\":\"ok\"}\n```"]));
        let g: Generated<Summary> = generate(&router(stub), Task::Summary, SUMMARY, "t")
            .await
            .unwrap();
        assert_eq!(
            g.value,
            Summary {
                summary: "ok".into()
            }
        );
        assert_eq!(g.calls.len(), 1);
    }

    #[tokio::test]
    async fn retries_once_with_the_error_then_succeeds() {
        let stub = Arc::new(Stub::texts(&["not json", "{\"summary\":\"fixed\"}"]));
        let g: Generated<Summary> = generate(&router(stub.clone()), Task::Summary, SUMMARY, "T")
            .await
            .unwrap();
        assert_eq!(g.value.summary, "fixed");
        assert_eq!(g.calls.len(), 2);
        let seen = stub.requests();
        assert_eq!(seen[0].prompt, "T");
        assert!(seen[1]
            .prompt
            .starts_with("T\n\nYour previous reply was not valid"));
        assert!(seen[1].prompt.ends_with("not json"));
    }

    #[tokio::test]
    async fn fails_after_second_bad_reply() {
        let stub = Arc::new(Stub::texts(&["{}", "{\"wrong\":1}"]));
        let r = generate::<Summary>(&router(stub.clone()), Task::Summary, SUMMARY, "t").await;
        assert!(
            matches!(r, Err(LlmError::Provider(m)) if m.starts_with("invalid JSON after retry"))
        );
        assert_eq!(stub.requests().len(), 2);
    }

    #[test]
    fn task_shapes_parse() {
        let a: ActionItems = serde_json::from_str(
            r#"{"action_items":[{"text":"x","owner":null,"due":"2026-01-02","source_ms":5}]}"#,
        )
        .unwrap();
        assert_eq!(a.action_items[0].due.as_deref(), Some("2026-01-02"));
        let c: Chapters =
            serde_json::from_str(r#"{"chapters":[{"start_ms":0,"title":"a"}]}"#).unwrap();
        assert_eq!(c.chapters.len(), 1);
        // every prompt names the JSON key its struct reads
        for (p, k) in [
            (SUMMARY, "summary"),
            (DECISIONS, "open_questions"),
            (CHAPTERS, "chapters"),
            (ACTION_ITEMS, "action_items"),
        ] {
            assert!(p.contains(&format!("\"{k}\"")), "{k}");
        }
    }
}
