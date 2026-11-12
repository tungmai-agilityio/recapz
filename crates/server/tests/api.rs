use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use recapz_core::db;
use recapz_server::{config::Config, routes};
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
    routes::router(cfg, Arc::new(Mutex::new(control)))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Option<String>, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned());
    let body =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    (status, cookie, body)
}

fn login(email: &str) -> Request<Body> {
    Request::post("/auth/dev-login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"email":"{email}"}}"#)))
        .unwrap()
}

#[tokio::test]
async fn health_reports_ok() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _, body) = send(
        &app(dir.path()),
        Request::get("/api/health").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, r#"{"status":"ok"}"#));
}

#[tokio::test]
async fn sign_in_is_invite_only_and_sessions_end_on_logout() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());

    let me = |c: &str| {
        Request::get("/api/me")
            .header(header::COOKIE, c)
            .body(Body::empty())
            .unwrap()
    };
    let (s, ..) = send(&app, Request::get("/api/me").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, ..) = send(&app, login("stranger@example.com")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, cookie, _) = send(&app, login("boss@example.com")).await;
    assert_eq!(s, StatusCode::OK);
    let cookie = cookie.unwrap();
    let (s, _, body) = send(&app, me(&cookie)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains(r#""email":"boss@example.com""#) && body.contains(r#""role":"admin""#));

    let logout = Request::post("/auth/logout")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    send(&app, logout).await;
    let (s, ..) = send(&app, me(&cookie)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn dev_login_is_off_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().into(),
        ..Config::default()
    };
    let control = db::open_control(dir.path()).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    let app = routes::router(cfg, Arc::new(Mutex::new(control)));
    let (s, ..) = send(&app, login("boss@example.com")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

fn get(path: &str, cookie: &str) -> Request<Body> {
    Request::get(path)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

fn upload(cookie: &str, title: &str) -> Request<Body> {
    let b = "XBOUNDARY";
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\n{title}\r\n--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFdata\r\n--{b}--\r\n"
    );
    Request::post("/api/uploads")
        .header(header::COOKIE, cookie)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={b}"),
        )
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn upload_creates_a_meeting_and_a_job_visible_only_to_its_owner() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();

    let (s, ..) = send(&app, upload(&boss, "Weekly sync")).await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, _, body) = send(&app, get("/api/meetings", &boss)).await;
    let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["title"], "Weekly sync");
    assert_eq!(rows[0]["status"], "uploaded");

    // A transcribe job was queued and the audio file was stored under the tenant folder.
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let jobs: i64 = t
        .query_row(
            "SELECT count(*) FROM jobs WHERE kind='transcribe'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(jobs, 1);
    let path: String = t
        .query_row("SELECT audio_path FROM meetings", [], |r| r.get(0))
        .unwrap();
    assert!(db::tenant_dir(dir.path(), &tid).join(path).exists());

    // Another member of the same workspace cannot see it.
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let (s, _, body) = send(&app, get("/api/meetings", &amy.unwrap())).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, "[]"));

    // Signed-out uploads are rejected.
    let (s, ..) = send(&app, upload("recapz_session=nope", "x")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn meeting_detail_and_transcript_follow_the_access_rule() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    send(&app, upload(&boss, "Weekly sync")).await;

    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let id: String = t
        .query_row("SELECT id FROM meetings", [], |r| r.get(0))
        .unwrap();
    t.execute_batch(&format!(
        "INSERT INTO segments (meeting_id, start_ms, end_ms, text) VALUES
           ('{id}', 2000, 3000, 'second'), ('{id}', 0, 2000, 'first')"
    ))
    .unwrap();
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();

    // Owner: detail plus segments in time order.
    let (s, _, body) = send(&app, get(&format!("/api/meetings/{id}"), &boss)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains(r#""title":"Weekly sync""#), "{body}");
    let (s, _, body) = send(&app, get(&format!("/api/meetings/{id}/transcript"), &boss)).await;
    assert_eq!(s, StatusCode::OK);
    let segs: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(segs.as_array().unwrap().len(), 2);
    assert_eq!(
        (segs[0]["text"].as_str(), segs[1]["text"].as_str()),
        (Some("first"), Some("second"))
    );

    // Another member gets 404 for both, as if it did not exist.
    for path in [
        format!("/api/meetings/{id}"),
        format!("/api/meetings/{id}/transcript"),
    ] {
        let (s, ..) = send(&app, get(&path, &amy)).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{path}");
    }
    // Unknown id and signed-out requests.
    let (s, ..) = send(&app, get("/api/meetings/nope", &boss)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, ..) = send(
        &app,
        get(&format!("/api/meetings/{id}"), "recapz_session=nope"),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Sharing with the workspace opens it up.
    t.execute("UPDATE meetings SET share='workspace'", [])
        .unwrap();
    let (s, ..) = send(&app, get(&format!("/api/meetings/{id}/transcript"), &amy)).await;
    assert_eq!(s, StatusCode::OK);
}
