use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use recapz_core::{
    db,
    embed::{to_blob, Embedder, HashEmbedder},
};
use recapz_llm::{stub::Stub, Completion, Price, Route, Router as Llm, Task};
use recapz_server::{config::Config, routes};
use rusqlite::params;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

fn app(dir: &std::path::Path, stub: Option<Arc<Stub>>) -> Router {
    let cfg = Config {
        data_dir: dir.into(),
        dev_login: true,
        ..Config::default()
    };
    let control = db::open_control(dir).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    let llm = stub.map(|s| {
        Arc::new(Llm::new().provider("p", s).route(
            Task::AskAnswer,
            Route {
                provider: "p".into(),
                model: "m".into(),
                max_tokens: 10,
                price: Price {
                    input: 1_000_000,
                    output: 0,
                },
            },
        ))
    });
    routes::router_with_llm(cfg, Arc::new(Mutex::new(control)), llm)
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Option<String>, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .find(|c| c.starts_with("recapz_session="));
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, cookie, String::from_utf8_lossy(&body).into())
}

fn login(email: &str) -> Request<Body> {
    Request::post("/auth/dev-login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"email":"{email}"}}"#)))
        .unwrap()
}

fn ask(cookie: &str, question: &str) -> Request<Body> {
    Request::post("/api/ask")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .body(Body::from(
            serde_json::json!({ "question": question }).to_string(),
        ))
        .unwrap()
}

/// The answer text rebuilt from the `token` events.
fn answer_text(sse: &str) -> String {
    let mut out = String::new();
    let mut lines = sse.lines();
    while let Some(l) = lines.next() {
        if l == "event: token" {
            let d: serde_json::Value =
                serde_json::from_str(lines.next().unwrap().trim_start_matches("data: ")).unwrap();
            out.push_str(d["text"].as_str().unwrap());
        }
    }
    out
}

fn reply(text: &str) -> Arc<Stub> {
    Arc::new(Stub::new(vec![Ok(Completion {
        text: text.into(),
        input_tokens: 100,
        output_tokens: 5,
    })]))
}

/// Two meetings owned by the boss: one private, one shared with the workspace.
fn seed(dir: &std::path::Path) -> (String, String) {
    let control = db::open_control(dir).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let t = db::open_tenant(dir, &tid).unwrap();
    let boss: String = control
        .query_row(
            "SELECT id FROM users WHERE email='boss@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    t.execute(
        "INSERT INTO members (id, email, role) VALUES (?1, 'boss@example.com', 'admin')",
        [&boss],
    )
    .unwrap();
    for (id, share, text) in [
        (
            "private",
            "attendees",
            "the secret acquisition price is nine million",
        ),
        (
            "shared",
            "workspace",
            "the launch date moved to May after the review",
        ),
    ] {
        t.execute(
            "INSERT INTO meetings (id, title, platform, status, owner_id, share) VALUES (?1, ?1, 'upload', 'ready', ?2, ?3)",
            params![id, boss, share],
        )
        .unwrap();
        t.execute(
            "INSERT INTO chunks (meeting_id, start_ms, end_ms, text, embedding) VALUES (?1, 90000, 150000, ?2, ?3)",
            params![id, text, to_blob(&HashEmbedder.embed(text).unwrap())],
        )
        .unwrap();
    }
    (tid, boss)
}

#[tokio::test]
async fn ask_never_reads_or_sends_another_members_private_meeting() {
    let dir = tempfile::tempdir().unwrap();
    let stub = reply(r#"{"answer":"May.","citations":[1]}"#);
    let app = app(dir.path(), Some(stub.clone()));
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    seed(dir.path());
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();

    // Amy asks about the private deal: nothing visible matches, so the model is never called.
    let (s, _, body) = send(&app, ask(&amy, "what is the acquisition price?")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        !body.contains("nine million") && !body.contains("private"),
        "{body}"
    );
    assert_eq!(answer_text(&body), recapz_llm::ask::NOT_FOUND);
    assert!(stub.requests().is_empty());

    // Amy asking about the shared meeting reaches the model with only that excerpt.
    let (s, _, body) = send(&app, ask(&amy, "when is the launch date?")).await;
    assert_eq!(s, StatusCode::OK);
    let sent = stub.requests();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].prompt.contains("launch date moved to May"));
    assert!(!sent[0].prompt.contains("nine million"));
    assert!(
        !sent[0].prompt.contains("shared"),
        "meeting titles are not sent to the model"
    );
    assert_eq!(answer_text(&body), "May.");
    assert!(body.contains(r#""meeting_id":"shared""#), "{body}");
    assert!(body.contains(r#""start_ms":90000"#), "{body}");
    assert!(body.trim_end().ends_with("event: done\ndata: {}"), "{body}");

    // The owner can find the private one.
    let stub2 = reply(r#"{"answer":"Nine million.","citations":[1]}"#);
    let app2 = self::app(dir.path(), Some(stub2.clone()));
    let (_, boss2, _) = send(&app2, login("boss@example.com")).await;
    let (_, _, body) = send(
        &app2,
        ask(&boss2.unwrap(), "what is the acquisition price?"),
    )
    .await;
    assert!(body.contains(r#""meeting_id":"private""#), "{body}");
    drop(boss);
}

#[tokio::test]
async fn ask_records_usage_and_rejects_bad_requests() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(
        dir.path(),
        Some(reply(r#"{"answer":"May.","citations":[1]}"#)),
    );
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let (tid, _) = seed(dir.path());

    let (s, ..) = send(&app, ask("recapz_session=nope", "x")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, ..) = send(&app, ask(&boss, "   ")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, ..) = send(&app, ask(&boss, &"x".repeat(1001))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, ..) = send(&app, ask(&boss, "when is the launch date?")).await;
    assert_eq!(s, StatusCode::OK);
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let (n, cost, input): (i64, i64, i64) = t
        .query_row(
            "SELECT COUNT(*), SUM(cost_micro_usd), SUM(input_tokens) FROM llm_usage WHERE task='ask_answer'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((n, cost, input), (1, 100, 100));
}

#[tokio::test]
async fn ask_without_a_language_model_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), None);
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let (s, ..) = send(&app, ask(&boss.unwrap(), "anything")).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn ask_is_rate_limited_per_member_before_any_model_call() {
    let dir = tempfile::tempdir().unwrap();
    let ans = r#"{"answer":"May.","citations":[1]}"#;
    let stub = Arc::new(Stub::texts(&[ans, ans, ans]));
    let cfg = Config {
        data_dir: dir.path().into(),
        dev_login: true,
        ask_per_minute: 2,
        ..Config::default()
    };
    let control = db::open_control(dir.path()).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    let llm = Arc::new(Llm::new().provider("p", stub.clone()).route(
        Task::AskAnswer,
        Route {
            provider: "p".into(),
            model: "m".into(),
            max_tokens: 10,
            price: Price {
                input: 0,
                output: 0,
            },
        },
    ));
    let app = routes::router_with_llm(cfg, Arc::new(Mutex::new(control)), Some(llm));
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    seed(dir.path());

    // A rejected request does not use quota.
    let (s, ..) = send(&app, ask(&boss, "  ")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    for _ in 0..2 {
        let (s, ..) = send(&app, ask(&boss, "when is the launch date?")).await;
        assert_eq!(s, StatusCode::OK);
    }
    let (s, _, body) = send(&app, ask(&boss, "when is the launch date?")).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(
        stub.requests().len(),
        2,
        "the limited ask never reached the model"
    );
}

/// Streams its reply in small pieces, or fails part way, like a real provider.
struct Pieces {
    pieces: Vec<&'static str>,
    fail_after: bool,
}

#[async_trait::async_trait]
impl recapz_llm::Provider for Pieces {
    async fn complete(
        &self,
        _: recapz_llm::CompletionRequest,
    ) -> Result<Completion, recapz_llm::LlmError> {
        unreachable!("Ask streams")
    }
    async fn stream(
        &self,
        _: recapz_llm::CompletionRequest,
        on_delta: recapz_llm::OnDelta<'_>,
    ) -> Result<Completion, recapz_llm::LlmError> {
        for p in &self.pieces {
            on_delta(p);
        }
        if self.fail_after {
            return Err(recapz_llm::LlmError::Provider("http 500".into()));
        }
        Ok(Completion {
            text: self.pieces.concat(),
            input_tokens: 100,
            output_tokens: 5,
        })
    }
}

fn streaming_app(dir: &std::path::Path, p: Pieces) -> Router {
    let cfg = Config {
        data_dir: dir.into(),
        dev_login: true,
        ..Config::default()
    };
    let control = db::open_control(dir).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    let llm = Arc::new(Llm::new().provider("p", Arc::new(p)).route(
        Task::AskAnswer,
        Route {
            provider: "p".into(),
            model: "m".into(),
            max_tokens: 10,
            price: Price {
                input: 1_000_000,
                output: 0,
            },
        },
    ));
    routes::router_with_llm(cfg, Arc::new(Mutex::new(control)), Some(llm))
}

#[tokio::test]
async fn ask_sends_one_token_event_per_text_piece_then_citations_and_done() {
    let dir = tempfile::tempdir().unwrap();
    let app = streaming_app(
        dir.path(),
        Pieces {
            pieces: vec![
                r#"{"answer":"Ra m"#,
                r#"ắt tháng "#,
                r#"5.","citations":[1]}"#,
            ],
            fail_after: false,
        },
    );
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let (tid, _) = seed(dir.path());
    let (s, _, sse) = send(&app, ask(&boss.unwrap(), "when is the launch date?")).await;
    assert_eq!(s, StatusCode::OK);
    let names: Vec<&str> = sse
        .lines()
        .filter_map(|l| l.strip_prefix("event: "))
        .collect();
    assert_eq!(
        names,
        ["token", "token", "token", "citations", "done"],
        "{sse}"
    );
    assert_eq!(answer_text(&sse), "Ra mắt tháng 5.");
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let n: i64 = t
        .query_row("SELECT COUNT(*) FROM llm_usage", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1, "usage is recorded once the stream ends");
}

#[tokio::test]
async fn ask_reports_a_failure_after_streaming_started_as_an_error_event() {
    let dir = tempfile::tempdir().unwrap();
    let app = streaming_app(
        dir.path(),
        Pieces {
            pieces: vec![r#"{"answer":"Half"#],
            fail_after: true,
        },
    );
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    seed(dir.path());
    let (s, _, sse) = send(&app, ask(&boss.unwrap(), "when is the launch date?")).await;
    assert_eq!(s, StatusCode::OK, "the status was sent before the failure");
    assert!(sse.contains("event: error"), "{sse}");
    assert!(
        !sse.contains("event: citations") && !sse.contains("http 500"),
        "{sse}"
    );
}

fn add_doc(t: &rusqlite::Connection, id: &str, owner: &str, llm_ok: bool, text: &str) {
    t.execute(
        "INSERT INTO documents (id, member_id, name, kind, size_bytes, path, llm_ok, created_at) VALUES (?1, ?2, ?1 || '.md', 'md', 1, 'p', ?3, 0)",
        params![id, owner, llm_ok as i64],
    )
    .unwrap();
    t.execute(
        "INSERT INTO document_chunks (document_id, seq, text) VALUES (?1, 0, ?2)",
        params![id, text],
    )
    .unwrap();
    if llm_ok {
        t.execute(
            "INSERT INTO document_vectors (chunk_id, document_id, embedder, embedding) VALUES (last_insert_rowid(), ?1, ?2, ?3)",
            params![id, HashEmbedder.id(), to_blob(&HashEmbedder.embed(text).unwrap())],
        )
        .unwrap();
    }
}

#[tokio::test]
async fn ask_searches_only_the_members_own_documents_and_never_sends_a_toggle_off_one() {
    let dir = tempfile::tempdir().unwrap();
    let stub = reply(r#"{"answer":"Net 30.","citations":[1]}"#);
    let app = app(dir.path(), Some(stub.clone()));
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let (tid, boss_id) = seed(dir.path());
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    let control = db::open_control(dir.path()).unwrap();
    let amy_id: String = control
        .query_row(
            "SELECT id FROM users WHERE email='amy@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    t.execute(
        "INSERT INTO members (id, email, role) VALUES (?1, 'amy@example.com', 'member') ON CONFLICT DO NOTHING",
        [&amy_id],
    )
    .unwrap();
    add_doc(&t, "bossdoc", &boss_id, true, "zebrafish retainer sixty");
    add_doc(&t, "amyoff", &amy_id, false, "walrus escrow ninety");
    add_doc(&t, "amyon", &amy_id, true, "ocelot payroll thirty");

    // Another member's document is neither found, cited nor sent.
    let (_, _, body) = send(&app, ask(&amy, "zebrafish retainer?")).await;
    assert_eq!(answer_text(&body), recapz_llm::ask::NOT_FOUND);
    assert!(
        !body.contains("bossdoc") && !body.contains("retainer sixty"),
        "{body}"
    );
    assert!(stub.requests().is_empty());

    // A toggle-off document is cited as a passage; its text never reaches the model.
    let (_, _, body) = send(&app, ask(&amy, "walrus escrow?")).await;
    assert_eq!(
        answer_text(&body),
        recapz_llm::ask::NOT_FOUND,
        "{body} {:?}",
        stub.requests()
    );
    assert!(
        stub.requests().is_empty(),
        "no model call for a toggle-off document"
    );
    assert!(body.contains(r#""document_id":"amyoff""#), "{body}");
    assert!(body.contains(r#""answered":false"#), "{body}");
    assert!(
        body.contains(r#""meeting_id":null"#) && body.contains(r#""start_ms":null"#),
        "{body}"
    );

    // A toggle-on document is sent and cited as the answer source.
    let (_, _, body) = send(&app, ask(&amy, "ocelot payroll?")).await;
    let sent = stub.requests();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].prompt.contains("payroll thirty"));
    assert!(
        !sent[0].prompt.contains("escrow ninety") && !sent[0].prompt.contains("retainer sixty")
    );
    assert_eq!(answer_text(&body), "Net 30.");
    assert!(
        body.contains(r#""document_id":"amyon""#) && body.contains(r#""answered":true"#),
        "{body}"
    );
    drop(boss);
}
