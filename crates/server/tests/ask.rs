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
    ask_with(cookie, serde_json::json!({ "question": question }))
}

fn ask_with(cookie: &str, body: serde_json::Value) -> Request<Body> {
    Request::post("/api/ask")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .body(Body::from(body.to_string()))
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
    assert!(
        body.trim_end().contains("event: done\ndata: {\"id\":\""),
        "the done event names the saved turn: {body}"
    );

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

#[tokio::test]
async fn ask_citations_carry_the_marker_number_speaker_duration_and_ribbon() {
    let dir = tempfile::tempdir().unwrap();
    let app = streaming_app(
        dir.path(),
        Pieces {
            pieces: vec![
                r#"{"answer":"Moved to May [1]. Not [5].","#,
                r#""citations":[1]}"#,
            ],
            fail_after: false,
        },
    );
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let (tid, _) = seed(dir.path());
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    for (id, name) in [("s1", "Ann"), ("s2", "Bo")] {
        t.execute(
            "INSERT INTO speakers (id, meeting_id, label, display_name) VALUES (?1, 'shared', ?1, ?2)",
            params![id, name],
        )
        .unwrap();
    }
    for (sp, a, b) in [("s1", 0, 100_000), ("s2", 100_000, 200_000)] {
        t.execute(
            "INSERT INTO segments (meeting_id, speaker_id, start_ms, end_ms, text) VALUES ('shared', ?1, ?2, ?3, 'x')",
            params![sp, a, b],
        )
        .unwrap();
    }
    let (s, _, sse) = send(&app, ask(&boss.unwrap(), "when is the launch date?")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(answer_text(&sse), "Moved to May [1]. Not .");
    let data = sse
        .split("event: citations\ndata: ")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap();
    let cites: serde_json::Value = serde_json::from_str(data).unwrap();
    let c = &cites[0];
    assert_eq!(cites.as_array().unwrap().len(), 1, "{data}");
    assert_eq!(
        (
            c["n"].clone(),
            c["speaker"].clone(),
            c["duration_ms"].clone()
        ),
        (1.into(), "Bo".into(), 200_000.into())
    );
    assert_eq!(
        c["ribbon"],
        serde_json::json!([{"speaker":0,"weight":30},{"speaker":1,"weight":30}])
    );
}

#[tokio::test]
async fn ask_filters_by_date_and_person_and_a_filter_never_widens_access() {
    let dir = tempfile::tempdir().unwrap();
    // One model call per search that finds something: amy twice, the boss once.
    let stub = Arc::new(Stub::new(
        (0..3)
            .map(|_| {
                Ok(Completion {
                    text: r#"{"answer":"May.","citations":[1]}"#.into(),
                    input_tokens: 100,
                    output_tokens: 5,
                })
            })
            .collect(),
    ));
    let app = app(dir.path(), Some(stub.clone()));
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let (tid, _) = seed(dir.path());
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    // 2026-11-15 12:00 UTC for the shared meeting; the private one stays at its default (1970).
    t.execute(
        "UPDATE meetings SET started_at = 1794744000000 WHERE id = 'shared'",
        [],
    )
    .unwrap();
    t.execute(
        "INSERT INTO speakers (id, meeting_id, label, display_name) VALUES ('p1','shared','Speaker 1','Linh'), ('p2','private','Speaker 1','Linh')",
        [],
    )
    .unwrap();
    let q = |who: &str, f: serde_json::Value| {
        let mut b = serde_json::json!({ "question": "when is the launch date?" });
        b.as_object_mut()
            .unwrap()
            .extend(f.as_object().unwrap().clone());
        ask_with(who, b)
    };
    let cited = |body: &str| body.contains(r#""meeting_id":"shared""#);

    let (s, _, body) = send(&app, q(&amy, serde_json::json!({}))).await;
    assert_eq!((s, cited(&body)), (StatusCode::OK, true));
    let (_, _, body) = send(
        &app,
        q(
            &amy,
            serde_json::json!({"from": "2026-11-15", "to": "2026-11-15", "speaker": " linh "}),
        ),
    )
    .await;
    assert!(cited(&body), "{body}");
    for f in [
        serde_json::json!({"from": "2026-11-16"}),
        serde_json::json!({"to": "2026-11-14"}),
        serde_json::json!({"speaker": "Minh"}),
    ] {
        let (s, _, body) = send(&app, q(&amy, f.clone())).await;
        assert_eq!((s, cited(&body)), (StatusCode::OK, false), "{f}");
    }
    // The private meeting matches the date and person filter but is still hidden from amy, and the
    // boss, who owns it, finds it.
    let pq = |who: &str| {
        ask_with(
            who,
            serde_json::json!({"question": "what is the acquisition price?", "to": "1970-01-02", "speaker": "Linh"}),
        )
    };
    let (_, _, body) = send(&app, pq(&amy)).await;
    assert!(
        !body.contains("private") && !body.contains("nine million"),
        "{body}"
    );
    let (_, _, body) = send(&app, pq(&boss)).await;
    assert!(body.contains(r#""meeting_id":"private""#), "{body}");
    // Bad dates are refused before any search.
    for bad in ["2026-02-30", "2026-1-5", "yesterday"] {
        let (s, ..) = send(&app, q(&boss, serde_json::json!({ "from": bad }))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
}

fn get_history(cookie: &str) -> Request<Body> {
    Request::get("/api/ask/history")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

fn delete(cookie: &str, path: &str, csrf: bool) -> Request<Body> {
    let mut r = Request::delete(path);
    if csrf {
        r = r
            .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
            .header("x-csrf-token", "t");
    } else {
        r = r.header(header::COOKIE, cookie);
    }
    r.body(Body::empty()).unwrap()
}

async fn history_of(app: &Router, cookie: &str) -> Vec<serde_json::Value> {
    let (s, _, body) = send(app, get_history(cookie)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    serde_json::from_str(&body).unwrap()
}

fn done_id(sse: &str) -> String {
    let line = sse
        .lines()
        .skip_while(|l| *l != "event: done")
        .nth(1)
        .unwrap();
    let d: serde_json::Value = serde_json::from_str(line.trim_start_matches("data: ")).unwrap();
    d["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_finished_answer_is_saved_and_listed_with_its_citations() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(
        dir.path(),
        Some(reply(r#"{"answer":"May.","citations":[1]}"#)),
    );
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    seed(dir.path());
    assert!(history_of(&app, &boss).await.is_empty());

    let (_, _, sse) = send(&app, ask(&boss, "when is the launch date?")).await;
    let id = done_id(&sse);

    let h = history_of(&app, &boss).await;
    assert_eq!(h.len(), 1);
    assert_eq!(h[0]["id"], id);
    assert_eq!(h[0]["question"], "when is the launch date?");
    assert_eq!(h[0]["answer"], "May.");
    assert_eq!(h[0]["citations"][0]["meeting_id"], "shared");
    assert_eq!(h[0]["citations"][0]["start_ms"], 90000);
}

#[tokio::test]
async fn a_failed_answer_is_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    let app = streaming_app(
        dir.path(),
        Pieces {
            pieces: vec!["{\"answer\":\"Ma"],
            fail_after: true,
        },
    );
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    seed(dir.path());
    let (_, _, sse) = send(&app, ask(&boss, "when is the launch date?")).await;
    assert!(sse.contains("event: error"), "{sse}");
    assert!(history_of(&app, &boss).await.is_empty());
}

#[tokio::test]
async fn history_is_private_and_hides_turns_citing_a_meeting_the_member_lost() {
    let dir = tempfile::tempdir().unwrap();
    let ans = r#"{"answer":"A.","citations":[1]}"#;
    let app = app(dir.path(), Some(Arc::new(Stub::texts(&[ans, ans]))));
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let (tid, _) = seed(dir.path());
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();

    let (_, _, sse) = send(&app, ask(&boss, "what is the acquisition price?")).await;
    let private_turn = done_id(&sse);
    let (_, _, sse) = send(&app, ask(&amy, "when is the launch date?")).await;
    let amy_turn = done_id(&sse);

    // Each member sees only their own, and cannot delete the other's.
    assert_eq!(history_of(&app, &amy).await.len(), 1);
    assert_eq!(history_of(&app, &amy).await[0]["id"], amy_turn);
    assert_eq!(history_of(&app, &boss).await.len(), 1);
    let (s, ..) = send(
        &app,
        delete(&boss, &format!("/api/ask/history/{amy_turn}"), true),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(history_of(&app, &amy).await.len(), 1);

    // The boss loses the private meeting: the turn is hidden, not deleted, and returns with access.
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    t.execute(
        "UPDATE meetings SET owner_id = NULL WHERE id = 'private'",
        [],
    )
    .unwrap();
    assert!(history_of(&app, &boss).await.is_empty());
    let rows: i64 = t
        .query_row("SELECT count(*) FROM ask_turns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 2, "hidden, not deleted");
    t.execute(
        "UPDATE meetings SET owner_id = (SELECT id FROM members WHERE email='boss@example.com') WHERE id = 'private'",
        [],
    )
    .unwrap();
    let h = history_of(&app, &boss).await;
    assert_eq!(h.len(), 1);
    assert_eq!(h[0]["id"], private_turn);
}

#[tokio::test]
async fn a_member_deletes_one_turn_or_clears_all_and_deletes_need_csrf() {
    let dir = tempfile::tempdir().unwrap();
    let ans = r#"{"answer":"May.","citations":[1]}"#;
    let app = app(dir.path(), Some(Arc::new(Stub::texts(&[ans, ans, ans]))));
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    seed(dir.path());
    let mut ids = vec![];
    for q in ["launch date one?", "launch date two?", "launch date three?"] {
        let (_, _, sse) = send(&app, ask(&boss, q)).await;
        ids.push(done_id(&sse));
    }
    assert_eq!(history_of(&app, &boss).await.len(), 3);

    let (s, ..) = send(
        &app,
        delete(&boss, &format!("/api/ask/history/{}", ids[0]), false),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "no csrf token");
    let (s, ..) = send(&app, delete(&boss, "/api/ask/history", false)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "no csrf token");
    assert_eq!(history_of(&app, &boss).await.len(), 3);

    let (s, ..) = send(
        &app,
        delete(&boss, &format!("/api/ask/history/{}", ids[1]), true),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, ..) = send(
        &app,
        delete(&boss, &format!("/api/ask/history/{}", ids[1]), true),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "already gone");
    let left: Vec<_> = history_of(&app, &boss)
        .await
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(left, [ids[0].clone(), ids[2].clone()]);

    let (s, ..) = send(&app, delete(&boss, "/api/ask/history", true)).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(history_of(&app, &boss).await.is_empty());
    let (s, ..) = send(&app, get_history("recapz_session=nope")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}
