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
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
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
        .header(header::COOKIE, cookie)
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
            params![id, text, to_blob(&HashEmbedder.embed(text))],
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
