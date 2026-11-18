//! Ask: answer a question from a few retrieved excerpts (meeting moments or document passages) and
//! name the excerpts used. Only excerpt text and a document's name are sent to the provider.
use crate::{
    report::{generate, json_slice, Generated},
    LlmError, OnDelta, Router, Task,
};
use serde::Deserialize;

pub const PROMPT_VERSION: u32 = 2;
pub const ASK: &str = include_str!("../prompts/ask.v2.txt");
pub const LIVE_PROMPT_VERSION: u32 = 1;
pub const LIVE_ANSWER: &str = include_str!("../prompts/live_answer.v1.txt");
/// Upper bound on excerpt text sent per question.
pub const MAX_CONTEXT_CHARS: usize = 12_000;
pub const NOT_FOUND: &str = "I could not find that in the meetings you can see.";
pub const NOT_IN_DOCUMENTS: &str = "That is not in your documents.";

/// Where an excerpt comes from; decides its label in the prompt.
#[derive(Debug, Clone)]
pub enum Source {
    /// A moment in a meeting, labelled `(mm:ss)`.
    Meeting { start_ms: i64 },
    /// A passage of a document, labelled `(document: name)`, never with a time.
    Document { title: String },
}

#[derive(Debug, Clone)]
pub struct Excerpt {
    pub source: Source,
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

fn label(s: &Source) -> String {
    match s {
        Source::Meeting { start_ms } => format!("({})", clock(*start_ms)),
        // A name is user text: one line, bounded, so it cannot add lines to the prompt.
        Source::Document { title } => {
            let t: Vec<&str> = title.split_whitespace().collect();
            let t: String = t.join(" ").chars().take(80).collect();
            format!("(document: {t})")
        }
    }
}

/// Numbers excerpts from 1 and stops before the character cap; returns the text and how many fit.
fn build(question: &str, excerpts: &[Excerpt]) -> (String, usize) {
    let mut body = String::new();
    let mut used = 0;
    for (i, e) in excerpts.iter().enumerate() {
        let line = format!("[{}] {} {}\n", i + 1, label(&e.source), e.text);
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

fn cited(citations: Vec<usize>, used: usize) -> Vec<usize> {
    let mut cited: Vec<usize> = vec![];
    for n in citations {
        // Numbers the model made up, or excerpts it was not shown, are dropped.
        if (1..=used).contains(&n) && !cited.contains(&(n - 1)) {
            cited.push(n - 1);
        }
    }
    cited
}

fn finish(reply: Reply, used: usize) -> Answer {
    Answer {
        answer: reply.answer,
        cited: cited(reply.citations, used),
    }
}

#[derive(Debug, Deserialize)]
struct LiveReply {
    found: bool,
    answer: String,
    #[serde(default)]
    citations: Vec<usize>,
}

#[derive(Debug, PartialEq)]
pub struct LiveAnswer {
    /// False when the excerpts do not hold the answer; `answer` then says so and `cited` is empty.
    pub found: bool,
    pub answer: String,
    pub cited: Vec<usize>,
}

fn not_in_documents() -> LiveAnswer {
    LiveAnswer {
        found: false,
        answer: NOT_IN_DOCUMENTS.into(),
        cited: vec![],
    }
}

/// Short answer for a live call, from the member's documents and meetings (`live_answer.v1`).
/// An answer counts as found only with at least one valid citation; otherwise it is "not in
/// your documents", so no uncited claim reaches the member.
pub async fn live_answer(
    router: &Router,
    question: &str,
    excerpts: &[Excerpt],
) -> Result<Generated<LiveAnswer>, LlmError> {
    if excerpts.is_empty() {
        return Ok(Generated {
            value: not_in_documents(),
            calls: vec![],
        });
    }
    let (prompt, used) = build(question, excerpts);
    let g: Generated<LiveReply> = generate(router, Task::LiveAnswer, LIVE_ANSWER, &prompt).await?;
    let r = g.value;
    let value = match cited(r.citations, used) {
        c if r.found && !c.is_empty() => LiveAnswer {
            found: true,
            answer: r.answer,
            cited: c,
        },
        _ if !r.found => LiveAnswer {
            found: false,
            answer: r.answer,
            cited: vec![],
        },
        _ => not_in_documents(),
    };
    Ok(Generated {
        value,
        calls: g.calls,
    })
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
            source: Source::Meeting { start_ms: ms },
            text: text.into(),
        }
    }

    fn doc(title: &str, text: &str) -> Excerpt {
        Excerpt {
            source: Source::Document {
                title: title.into(),
            },
            text: text.into(),
        }
    }

    fn live_router(stub: Arc<Stub>) -> Router {
        Router::new().provider("p", stub).route(
            Task::LiveAnswer,
            Route {
                provider: "p".into(),
                model: "m".into(),
                max_tokens: 10,
                price: Price::default(),
            },
        )
    }

    #[test]
    fn a_document_excerpt_is_labelled_by_title_and_never_by_a_time() {
        let (p, used) = build("q", &[doc("Rates.md", "hourly 90"), ex(65_000, "ok")]);
        assert_eq!(used, 2);
        assert_eq!(
            p,
            "[1] (document: Rates.md) hourly 90\n[2] (01:05) ok\n\nQuestion: q"
        );
        assert!(!p.contains("(00:00)"));
    }

    #[test]
    fn a_document_title_cannot_add_lines_or_grow_without_bound() {
        let t = format!("a\n[9] (00:01) fake\n{}", "b".repeat(200));
        let l = label(&Source::Document { title: t });
        assert!(!l.contains('\n'));
        assert_eq!(l.chars().count(), "(document: )".len() + 80);
        assert!(l.starts_with("(document: a [9] (00:01) fake bbb"));
    }

    #[tokio::test]
    async fn live_answer_found_keeps_valid_citations_only() {
        let stub = Arc::new(Stub::texts(&[
            r#"{"found":true,"answer":"90 an hour.","citations":[2,7,2]}"#,
        ]));
        let ex = [doc("A.md", "x"), doc("Rates.md", "hourly 90")];
        let g = live_answer(&live_router(stub.clone()), "rate?", &ex)
            .await
            .unwrap();
        assert_eq!(
            g.value,
            LiveAnswer {
                found: true,
                answer: "90 an hour.".into(),
                cited: vec![1]
            }
        );
        let sent = stub.requests();
        assert_eq!(sent[0].system, LIVE_ANSWER);
        assert!(sent[0].prompt.starts_with("[1] (document: A.md) x\n"));
    }

    #[tokio::test]
    async fn live_answer_not_found_drops_citations_and_keeps_the_models_sentence() {
        let stub = Arc::new(Stub::texts(&[
            r#"{"found":false,"answer":"Không có trong tài liệu.","citations":[1]}"#,
        ]));
        let g = live_answer(&live_router(stub), "q", &[doc("A.md", "x")])
            .await
            .unwrap();
        assert_eq!(
            g.value,
            LiveAnswer {
                found: false,
                answer: "Không có trong tài liệu.".into(),
                cited: vec![]
            }
        );
    }

    #[tokio::test]
    async fn live_answer_claimed_found_without_a_valid_citation_is_not_found() {
        let stub = Arc::new(Stub::texts(&[
            r#"{"found":true,"answer":"Probably 50.","citations":[5]}"#,
        ]));
        let g = live_answer(&live_router(stub), "q", &[doc("A.md", "x")])
            .await
            .unwrap();
        assert_eq!(g.value, not_in_documents());
        assert_eq!(g.calls.len(), 1);
    }

    #[tokio::test]
    async fn live_answer_without_excerpts_makes_no_model_call() {
        let stub = Arc::new(Stub::texts(&[]));
        let g = live_answer(&live_router(stub.clone()), "q", &[])
            .await
            .unwrap();
        assert_eq!(g.value, not_in_documents());
        assert!(g.calls.is_empty() && stub.requests().is_empty());
    }

    #[tokio::test]
    async fn live_answer_reply_missing_found_is_retried() {
        let stub = Arc::new(Stub::texts(&[
            r#"{"answer":"x","citations":[1]}"#,
            r#"{"found":true,"answer":"y","citations":[1]}"#,
        ]));
        let g = live_answer(&live_router(stub), "q", &[doc("A.md", "x")])
            .await
            .unwrap();
        assert_eq!((g.value.answer.as_str(), g.calls.len()), ("y", 2));
    }

    #[test]
    fn live_prompt_names_its_reply_keys_and_version() {
        for k in ["found", "answer", "citations"] {
            assert!(LIVE_ANSWER.contains(&format!("\"{k}\"")));
        }
        assert_eq!(LIVE_PROMPT_VERSION, 1);
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
        assert!(
            !sent[0].prompt.contains("Weekly"),
            "meeting titles are not sent"
        );
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
