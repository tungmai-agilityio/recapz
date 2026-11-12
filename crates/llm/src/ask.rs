//! Ask: answer a question from a few retrieved transcript excerpts and name the excerpts used.
//! Only excerpt text is sent to the provider, never a whole transcript.
use crate::{
    report::{generate, json_slice, Generated},
    LlmError, OnDelta, Router, Task,
};
use serde::Deserialize;

pub const PROMPT_VERSION: u32 = 1;
pub const ASK: &str = include_str!("../prompts/ask.v1.txt");
/// Upper bound on excerpt text sent per question.
pub const MAX_CONTEXT_CHARS: usize = 12_000;
pub const NOT_FOUND: &str = "I could not find that in the meetings you can see.";

#[derive(Debug, Clone)]
pub struct Excerpt {
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
        let line = format!("[{}] ({}) {}\n", i + 1, clock(e.start_ms), e.text);
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
    Ok(Generated {
        value: finish(g.value, used),
        calls: g.calls,
    })
}

fn finish(reply: Reply, used: usize) -> Answer {
    let mut cited: Vec<usize> = vec![];
    for n in reply.citations {
        // Numbers the model made up, or excerpts it was not shown, are dropped.
        if (1..=used).contains(&n) && !cited.contains(&(n - 1)) {
            cited.push(n - 1);
        }
    }
    Answer {
        answer: reply.answer,
        cited,
    }
}

/// `answer`, passing the answer text to `on_text` while the model writes it.
/// A reply that turns out not to be valid JSON is retried (without streaming) only if no text
/// was sent yet; after that the text cannot be taken back, so the call fails instead.
pub async fn answer_stream(
    router: &Router,
    question: &str,
    excerpts: &[Excerpt],
    on_text: OnDelta<'_>,
) -> Result<Generated<Answer>, LlmError> {
    if excerpts.is_empty() {
        on_text(NOT_FOUND);
        return answer(router, question, excerpts).await;
    }
    let (prompt, used) = build(question, excerpts);
    let (mut text, mut sent) = (AnswerText::default(), false);
    let out = router
        .run_stream(Task::AskAnswer, ASK, &prompt, &mut |d| {
            let t = text.feed(d);
            if !t.is_empty() {
                sent = true;
                on_text(&t);
            }
        })
        .await?;
    match serde_json::from_str::<Reply>(json_slice(&out.completion.text)) {
        Ok(reply) => Ok(Generated {
            value: finish(reply, used),
            calls: vec![out],
        }),
        Err(_) if sent => Err(LlmError::Provider("reply was not valid JSON".into())),
        Err(_) => {
            let mut g = answer(router, question, excerpts).await?;
            on_text(&g.value.answer);
            g.calls.insert(0, out);
            Ok(g)
        }
    }
}

#[derive(Default)]
enum Pos {
    #[default]
    Key,
    Colon,
    Quote,
    Text,
    Esc,
    Hex(u32, u8),
    Done,
}

/// Pulls the decoded text of the `"answer"` string out of a JSON reply as it streams in, so the
/// reader sees words before the closing brace and the citations arrive.
#[derive(Default)]
pub struct AnswerText {
    pos: Pos,
    /// Last eight characters seen while looking for the key.
    head: String,
    high: Option<u32>,
}

impl AnswerText {
    /// Answer text newly completed by `chunk`.
    pub fn feed(&mut self, chunk: &str) -> String {
        let mut out = String::new();
        for c in chunk.chars() {
            match self.pos {
                Pos::Key => {
                    self.head.push(c);
                    if self.head.chars().count() > 8 {
                        self.head.remove(0);
                        while !self.head.is_char_boundary(0) {
                            self.head.remove(0);
                        }
                    }
                    if self.head == "\"answer\"" {
                        self.pos = Pos::Colon;
                    }
                }
                Pos::Colon if c.is_whitespace() => {}
                Pos::Colon => {
                    self.head.clear();
                    self.pos = if c == ':' { Pos::Quote } else { Pos::Key };
                }
                Pos::Quote if c.is_whitespace() => {}
                Pos::Quote => self.pos = if c == '"' { Pos::Text } else { Pos::Done },
                Pos::Text => match c {
                    '"' => self.pos = Pos::Done,
                    '\\' => self.pos = Pos::Esc,
                    _ => out.push(c),
                },
                Pos::Esc => {
                    self.pos = Pos::Text;
                    match c {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'u' => self.pos = Pos::Hex(0, 0),
                        other => out.push(other),
                    }
                }
                Pos::Hex(acc, n) => {
                    let acc = acc * 16 + c.to_digit(16).unwrap_or(0);
                    if n < 3 {
                        self.pos = Pos::Hex(acc, n + 1);
                        continue;
                    }
                    self.pos = Pos::Text;
                    match (self.high.take(), acc) {
                        (_, 0xD800..=0xDBFF) => self.high = Some(acc),
                        (Some(h), 0xDC00..=0xDFFF) => out.push(
                            char::from_u32(0x10000 + ((h - 0xD800) << 10) + (acc - 0xDC00))
                                .unwrap_or('\u{FFFD}'),
                        ),
                        _ => out.push(char::from_u32(acc).unwrap_or('\u{FFFD}')),
                    }
                }
                Pos::Done => {}
            }
        }
        out
    }
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

    fn ex(ms: i64, text: &str) -> Excerpt {
        Excerpt {
            start_ms: ms,
            text: text.into(),
        }
    }

    #[tokio::test]
    async fn cites_only_shown_excerpts_once_each_in_order() {
        let stub = Arc::new(Stub::texts(&[
            r#"{"answer":"Friday.","citations":[2,2,9,0,1]}"#,
        ]));
        let ex = [ex(65_000, "ships Friday"), ex(0, "ok")];
        let g = answer(&router(stub.clone()), "when?", &ex).await.unwrap();
        assert_eq!(g.value.answer, "Friday.");
        assert_eq!(g.value.cited, vec![1, 0]);
        let sent = stub.requests();
        assert_eq!(sent[0].system, ASK);
        assert!(!sent[0].prompt.contains("Weekly"), "titles are not sent");
        assert!(sent[0]
            .prompt
            .starts_with("[1] (01:05) ships Friday\n[2] (00:00) ok\n"));
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
        let ex = [ex(0, &big), ex(0, &big)];
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

    #[test]
    fn answer_text_decodes_the_answer_from_pieces_cut_anywhere() {
        let json = r#"{"citations": [], "answer" :  "Ra mắt \"thứ Sáu\"\nở \u0111\u00e2y \ud83d\ude00 \\ ok", "x": "no"}"#;
        let want = "Ra mắt \"thứ Sáu\"\nở đây 😀 \\ ok";
        for size in [1, 2, 5, json.len()] {
            let mut t = AnswerText::default();
            let chars: Vec<char> = json.chars().collect();
            let got: String = chars
                .chunks(size)
                .map(|p| t.feed(&p.iter().collect::<String>()))
                .collect();
            assert_eq!(got, want, "piece size {size}");
        }
    }

    #[test]
    fn answer_text_ignores_a_non_string_answer() {
        let mut t = AnswerText::default();
        assert_eq!(t.feed(r#"{"answer": null, "citations": [1]}"#), "");
    }

    #[tokio::test]
    async fn stream_sends_answer_text_then_returns_cited_excerpts_and_usage() {
        let stub = Arc::new(Stub::texts(&[r#"{"answer":"Friday.","citations":[2,9]}"#]));
        let ex = [ex(0, "ok"), ex(65_000, "ships Friday")];
        let mut got = String::new();
        let g = answer_stream(&router(stub), "when?", &ex, &mut |t| got.push_str(t))
            .await
            .unwrap();
        assert_eq!(
            (got.as_str(), g.value.answer.as_str()),
            ("Friday.", "Friday.")
        );
        assert_eq!(g.value.cited, vec![1]);
        assert_eq!(g.calls.len(), 1);
    }

    #[tokio::test]
    async fn stream_with_bad_json_retries_when_nothing_was_sent() {
        let stub = Arc::new(Stub::texts(&[
            "not json",
            r#"{"answer":"Fine.","citations":[1]}"#,
        ]));
        let mut got = String::new();
        let g = answer_stream(&router(stub), "q", &[ex(0, "a")], &mut |t| got.push_str(t))
            .await
            .unwrap();
        assert_eq!(
            (got.as_str(), g.value.cited.as_slice()),
            ("Fine.", &[0][..])
        );
        assert_eq!(g.calls.len(), 2, "both calls are billed");
    }

    #[tokio::test]
    async fn stream_fails_if_the_reply_breaks_after_text_was_sent() {
        let stub = Arc::new(Stub::texts(&[r#"{"answer":"Half"#]));
        let mut got = String::new();
        let e = answer_stream(&router(stub), "q", &[ex(0, "a")], &mut |t| got.push_str(t)).await;
        assert!(e.is_err());
        assert_eq!(got, "Half");
    }

    #[tokio::test]
    async fn stream_with_no_excerpts_says_not_found_without_a_model_call() {
        let stub = Arc::new(Stub::texts(&[]));
        let mut got = String::new();
        answer_stream(&router(stub.clone()), "q", &[], &mut |t| got.push_str(t))
            .await
            .unwrap();
        assert_eq!(got, NOT_FOUND);
        assert!(stub.requests().is_empty());
    }
}
