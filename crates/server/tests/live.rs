use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use recapz_core::db;
use recapz_server::{
    config::Config,
    live::{hub, Msg},
    routes,
};
use rusqlite::params;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

fn app(dir: &std::path::Path) -> Router {
    let cfg = Config {
        data_dir: dir.into(),
        dev_login: true,
        ..Config::default()
    };
    let control = db::open_control(dir).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    routes::router_with_llm(cfg, Arc::new(Mutex::new(control)), None)
}

async fn login(app: &Router, email: &str) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::post("/auth/dev-login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(r#"{{"email":"{email}"}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .find(|c| c.starts_with("recapz_session="))
        .unwrap()
}

/// Two recording meetings owned by the boss, one private and one shared with the workspace; amy is a member.
fn seed(dir: &std::path::Path) {
    let control = db::open_control(dir).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let boss: String = control
        .query_row(
            "SELECT id FROM users WHERE email='boss@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let t = db::open_tenant(dir, &tid).unwrap();
    t.execute(
        "INSERT INTO members (id, email, role) VALUES (?1, 'boss@example.com', 'admin')",
        [&boss],
    )
    .unwrap();
    for (id, share) in [("live-private", "attendees"), ("live-shared", "workspace")] {
        t.execute(
            "INSERT INTO meetings (id, title, platform, status, owner_id, share) VALUES (?1, ?1, 'meet', 'recording', ?2, ?3)",
            params![id, boss, share],
        )
        .unwrap();
    }
}

fn live(cookie: &str, id: &str) -> Request<Body> {
    Request::get(format!("/api/meetings/{id}/live"))
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

/// Reads the stream until `done` is seen in the text so far, returning everything read.
async fn read_until(body: &mut Body, done: &str) -> String {
    let mut out = String::new();
    while !out.contains(done) {
        let f = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
            .await
            .expect("the stream stalled")
            .expect("the stream ended early")
            .unwrap();
        out.push_str(&String::from_utf8_lossy(&f.into_data().unwrap()));
    }
    out
}

#[tokio::test]
async fn members_with_access_get_the_live_transcript_and_others_get_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let boss = login(&app, "boss@example.com").await;
    seed(dir.path());
    let amy = login(&app, "amy@example.com").await;
    for id in ["live-private", "live-shared"] {
        hub().start(id);
        hub().push(
            id,
            Msg::Final {
                at_ms: 1500,
                text: "the price is nine million".into(),
            },
        );
    }

    // Amy cannot see the private meeting, live or not: same answer as a meeting that does not exist.
    let res = app
        .clone()
        .oneshot(live(&amy, "live-private"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert!(res
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .is_empty());
    let res = app.clone().oneshot(live(&amy, "nope")).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = app
        .clone()
        .oneshot(live("recapz_session=bad", "live-shared"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // The owner gets what was said so far, then new text as it arrives.
    let res = app
        .clone()
        .oneshot(live(&boss, "live-shared"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "text/event-stream");
    let mut body = res.into_body();
    let line = "event: final\ndata: {\"at_ms\":1500,\"text\":\"the price is nine million\"}\n\n";
    assert_eq!(read_until(&mut body, "\n\n").await, line);
    hub().push("live-shared", Msg::Partial("and the".into()));
    assert_eq!(
        read_until(&mut body, "\n\n").await,
        "event: partial\ndata: {\"text\":\"and the\"}\n\n"
    );
    hub().push("live-shared", Msg::Unavailable);
    assert_eq!(
        read_until(&mut body, "\n\n").await,
        "event: unavailable\ndata: {}\n\n"
    );

    // A workspace member joining late gets the line, the partial and the failure notice.
    let res = app
        .clone()
        .oneshot(live(&amy, "live-shared"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body();
    let got = read_until(&mut body, "event: unavailable\ndata: {}\n\n").await;
    assert_eq!(
        got,
        format!("{line}event: partial\ndata: {{\"text\":\"and the\"}}\n\nevent: unavailable\ndata: {{}}\n\n")
    );

    // Ending the call (or an opt-out) closes the stream and drops the transcript.
    let res = app
        .clone()
        .oneshot(live(&boss, "live-private"))
        .await
        .unwrap();
    let mut body = res.into_body();
    read_until(&mut body, "nine million").await;
    hub().clear("live-private");
    read_until(&mut body, "event: end\ndata: {}\n\n").await;
    let res = app
        .clone()
        .oneshot(live(&boss, "live-private"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
