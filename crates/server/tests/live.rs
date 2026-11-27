use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use recapz_core::db;
use recapz_server::{
    config::Config,
    live::{hub, Control, Msg, Who},
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
                who: None,
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
    // A line with a speaker carries the number (its colour), the name to show and how long it took.
    hub().push(
        "live-shared",
        Msg::Final {
            at_ms: 4000,
            text: "agreed".into(),
            who: Some(Who {
                n: 1,
                label: "Ann".into(),
                ms: 900,
            }),
        },
    );
    let who_line = "event: final\ndata: {\"at_ms\":4000,\"label\":\"Ann\",\"ms\":900,\"speaker\":1,\"text\":\"agreed\"}\n\n";
    assert_eq!(read_until(&mut body, "\n\n").await, who_line);

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
        format!("{line}{who_line}event: unavailable\ndata: {{}}\n\n")
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

fn post(path: &str, cookie: &str) -> Request<Body> {
    Request::post(path)
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn only_the_owner_or_an_admin_pauses_resumes_or_stops_a_live_call() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let boss = login(&app, "boss@example.com").await;
    seed(dir.path());
    let amy = login(&app, "amy@example.com").await;
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    // Amy owns "ctl-amy"; "ctl-shared" belongs to the boss and is visible to her.
    let amy_id: String = control
        .query_row(
            "SELECT id FROM users WHERE email='amy@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    t.execute(
        "INSERT INTO members (id, email, role) VALUES (?1, 'amy@example.com', 'member') ON CONFLICT DO NOTHING",
        [&amy_id],
    )
    .unwrap();
    let boss_id: String = t
        .query_row(
            "SELECT id FROM members WHERE email='boss@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for (id, owner, share) in [
        ("ctl-amy", amy_id.as_str(), "workspace"),
        ("ctl-shared", boss_id.as_str(), "workspace"),
        ("ctl-boss", boss_id.as_str(), "attendees"),
    ] {
        t.execute(
            "INSERT INTO meetings (id, title, platform, status, owner_id, share) VALUES (?1, ?1, 'meet', 'recording', ?2, ?3)",
            params![id, owner, share],
        )
        .unwrap();
        hub().start(id);
    }
    let status = |req: Request<Body>| {
        let app = app.clone();
        async move { app.oneshot(req).await.unwrap().status() }
    };
    let act = |id: &str, what: &str| format!("/api/meetings/{id}/live/{what}");

    // Amy sees the boss's workspace-shared call but does not own it: 403, nothing changes. The boss's
    // private call is a 404 for her.
    assert_eq!(
        status(post(&act("ctl-shared", "pause"), &amy)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(post(&act("ctl-boss", "stop"), &amy)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(hub().control("ctl-shared"), Default::default());
    assert_eq!(hub().control("ctl-boss"), Default::default());
    // Signed out, and no CSRF token: refused before anything runs.
    assert_eq!(
        status(post(&act("ctl-amy", "pause"), "recapz_session=nope")).await,
        StatusCode::UNAUTHORIZED
    );
    let no_csrf = Request::post(act("ctl-amy", "pause"))
        .header(header::COOKIE, &amy)
        .body(Body::empty())
        .unwrap();
    assert_eq!(status(no_csrf).await, StatusCode::FORBIDDEN);

    // The owner controls her own call; repeats are harmless.
    for _ in 0..2 {
        assert_eq!(
            status(post(&act("ctl-amy", "pause"), &amy)).await,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        hub().control("ctl-amy"),
        Control {
            paused: true,
            stop: false
        }
    );
    assert_eq!(
        status(post(&act("ctl-amy", "resume"), &amy)).await,
        StatusCode::NO_CONTENT
    );
    assert!(!hub().control("ctl-amy").paused);
    // An admin may stop a call they do not own (once it is shared with the workspace so they see it).
    assert_eq!(
        status(post(&act("ctl-amy", "stop"), &boss)).await,
        StatusCode::NO_CONTENT
    );
    assert!(hub().control("ctl-amy").stop);
    // A call that is not live (never started, or already cleared) is a 404.
    hub().clear("ctl-shared");
    assert_eq!(
        status(post(&act("ctl-shared", "stop"), &boss)).await,
        StatusCode::NOT_FOUND
    );
    hub().clear("ctl-amy");
    hub().clear("ctl-boss");
}
