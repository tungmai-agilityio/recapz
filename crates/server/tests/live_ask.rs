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
use recapz_llm::{stub::Stub, Price, Route, Router as Llm, Task};
use recapz_server::{
    config::Config,
    live::{hub, Msg},
    routes,
};
use rusqlite::params;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

fn app(dir: &std::path::Path, stub: Option<Arc<Stub>>, per_minute: u32) -> Router {
    let cfg = Config {
        data_dir: dir.into(),
        dev_login: true,
        ask_per_minute: per_minute,
        ..Config::default()
    };
    let control = db::open_control(dir).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let llm = stub.map(|s| {
        let route = Route {
            provider: "p".into(),
            model: "m".into(),
            max_tokens: 10,
            price: Price {
                input: 1_000_000,
                output: 0,
            },
        };
        Arc::new(
            Llm::new()
                .provider("p", s)
                .route(Task::LiveAnswer, route.clone())
                .route(Task::AskAnswer, route),
        )
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

async fn login(app: &Router, email: &str) -> String {
    let (_, cookie, _) = send(
        app,
        Request::post("/auth/dev-login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(r#"{{"email":"{email}"}}"#)))
            .unwrap(),
    )
    .await;
    cookie.unwrap()
}

fn live_ask(cookie: &str, meeting: &str, body: serde_json::Value) -> Request<Body> {
    Request::post(format!("/api/meetings/{meeting}/live-ask"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn say(meeting: &str, text: &str) {
    hub().push(
        meeting,
        Msg::Final {
            at_ms: 0,
            text: text.into(),
            who: None,
        },
    );
}

fn user(control: &rusqlite::Connection, email: &str) -> String {
    control
        .query_row("SELECT id FROM users WHERE email=?1", [email], |r| r.get(0))
        .unwrap()
}

/// Call after both have logged in. Makes both members and gives the boss two recording meetings.
/// Returns the tenant id and both user ids.
fn seed(dir: &std::path::Path, meetings: [(&str, &str); 2]) -> (String, String, String) {
    let control = db::open_control(dir).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let (boss, amy) = (
        user(&control, "boss@example.com"),
        user(&control, "amy@example.com"),
    );
    let t = db::open_tenant(dir, &tid).unwrap();
    for (id, email, role) in [
        (&boss, "boss@example.com", "admin"),
        (&amy, "amy@example.com", "member"),
    ] {
        t.execute(
            "INSERT INTO members (id, email, role) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
            params![id, email, role],
        )
        .unwrap();
    }
    for (id, share) in meetings {
        t.execute(
            "INSERT INTO meetings (id, title, platform, status, owner_id, share) VALUES (?1, ?1, 'meet', 'recording', ?2, ?3)",
            params![id, boss, share],
        )
        .unwrap();
    }
    (tid, boss, amy)
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

fn found(answer: &str) -> Arc<Stub> {
    Arc::new(Stub::texts(&[&format!(
        r#"{{"found":true,"answer":"{answer}","citations":[1]}}"#
    )]))
}

#[tokio::test]
async fn a_live_question_is_answered_from_the_askers_own_sendable_documents_only() {
    let dir = tempfile::tempdir().unwrap();
    let stub = found("Net 30.");
    let app = app(dir.path(), Some(stub.clone()), 30);
    let boss = login(&app, "boss@example.com").await;
    let amy = login(&app, "amy@example.com").await;
    let (tid, boss_id, amy_id) = seed(
        dir.path(),
        [("la1-private", "attendees"), ("la1-shared", "workspace")],
    );
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    add_doc(&t, "bossdoc", &boss_id, true, "zebrafish retainer sixty");
    add_doc(&t, "amyoff", &amy_id, false, "walrus escrow ninety");
    add_doc(&t, "amyon", &amy_id, true, "ocelot payroll thirty");
    for m in ["la1-private", "la1-shared"] {
        hub().start(m);
    }
    say("la1-shared", "what are the ocelot payroll terms?");
    let ask = |who: &str, m: &str, b| live_ask(who, m, b);

    // Not allowed or not live: the same answer as a meeting that does not exist, and no model call.
    for (who, m) in [(&amy, "la1-private"), (&amy, "nope"), (&boss, "la1-gone")] {
        let (s, _, body) = send(&app, ask(who, m, serde_json::json!({}))).await;
        assert_eq!((s, body.as_str()), (StatusCode::NOT_FOUND, ""));
    }
    let (s, ..) = send(
        &app,
        ask("recapz_session=bad", "la1-shared", serde_json::json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(stub.requests().is_empty());

    // Amy asks in a call she can see: her sendable document answers, nothing else is sent.
    let (s, _, body) = send(&app, ask(&amy, "la1-shared", serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let sent = stub.requests();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].prompt.contains("ocelot payroll thirty"),
        "{}",
        sent[0].prompt
    );
    for leaked in ["zebrafish", "walrus", "amyon", "amyoff", "bossdoc", "la1-"] {
        assert!(
            !sent[0].prompt.contains(leaked),
            "{leaked} reached the model: {}",
            sent[0].prompt
        );
    }
    assert!(
        sent[0]
            .prompt
            .contains("what are the ocelot payroll terms?"),
        "the live transcript is the question"
    );
    assert!(
        body.starts_with("event: token\ndata: {\"text\":\"Net 30.\"}\n\nevent: citations\n"),
        "{body}"
    );
    assert!(
        body.contains(r#""document_id":"amyon""#) && body.contains(r#""answered":true"#),
        "{body}"
    );
    assert!(body.trim_end().ends_with("event: done\ndata: {}"), "{body}");
    let n: i64 = t
        .query_row(
            "SELECT COUNT(*) FROM llm_usage WHERE task='live_answer'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);

    // (`at_ms` names a line that was not said, so only the typed text is the question.)
    // A toggle-off document of hers is shown as a passage and never sent; the boss's is not found at all.
    let (_, _, body) = send(
        &app,
        ask(
            &amy,
            "la1-shared",
            serde_json::json!({"text": "walrus escrow zebrafish retainer?", "at_ms": 999}),
        ),
    )
    .await;
    assert_eq!(
        stub.requests().len(),
        1,
        "the toggle-off passage made no model call"
    );
    assert!(
        body.contains(r#""document_id":"amyoff""#) && body.contains(r#""answered":false"#),
        "{body}"
    );
    assert!(body.contains(recapz_llm::ask::NOT_IN_DOCUMENTS), "{body}");
    assert!(
        !body.contains("bossdoc") && !body.contains("retainer sixty"),
        "{body}"
    );
}

#[tokio::test]
async fn a_live_ask_needs_something_said_a_sane_size_and_a_model_and_shares_the_ask_quota() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(Stub::texts(&[]));
    let app = app(dir.path(), Some(stub.clone()), 2);
    let boss = login(&app, "boss@example.com").await;
    login(&app, "amy@example.com").await;
    seed(dir.path(), [("la2-a", "attendees"), ("la2-b", "attendees")]);
    hub().start("la2-a");
    let ask = |b| live_ask(&boss, "la2-a", b);

    // Rejected requests do not use quota.
    let (s, ..) = send(&app, ask(serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "nothing said yet");
    let (s, ..) = send(&app, ask(serde_json::json!({"text": "x".repeat(1001)}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // One Ask and one live ask use the same two-a-minute quota.
    let (s, ..) = send(
        &app,
        Request::post("/api/ask")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, format!("{boss}; recapz_csrf=t"))
            .header("x-csrf-token", "t")
            .body(Body::from(r#"{"question":"anything"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, ..) = send(&app, ask(serde_json::json!({"text": "hello?"}))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, ..) = send(&app, ask(serde_json::json!({"text": "hello?"}))).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert!(stub.requests().is_empty(), "no excerpts, so no model call");

    let dir2 = tempfile::tempdir().unwrap();
    let app = self::app(dir2.path(), None, 30);
    let boss = login(&app, "boss@example.com").await;
    login(&app, "amy@example.com").await;
    seed(
        dir2.path(),
        [("la2-c", "attendees"), ("la2-d", "attendees")],
    );
    hub().start("la2-c");
    let (s, ..) = send(
        &app,
        live_ask(&boss, "la2-c", serde_json::json!({"text": "hi"})),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
}
