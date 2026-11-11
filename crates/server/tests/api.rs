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
    for t in db::tenant_ids(&control).unwrap() {
        db::open_tenant(dir, &t).unwrap();
    }
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

fn get(path: &str, cookie: &str) -> Request<Body> {
    Request::get(path)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

fn upload(cookie: &str) -> Request<Body> {
    let b = "XBOUNDARY";
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nWeekly sync\r\n--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFdata\r\n--{b}--\r\n"
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
async fn health_auth_and_upload_flow() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());

    let (s, _, body) = send(
        &app,
        Request::get("/api/health").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, r#"{"status":"ok"}"#));

    // Signed out and uninvited users are rejected.
    let (s, ..) = send(
        &app,
        Request::get("/api/meetings").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, ..) = send(&app, login("stranger@example.com")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Invited admin signs in, uploads, and sees the meeting.
    let (s, cookie, _) = send(&app, login("boss@example.com")).await;
    assert_eq!(s, StatusCode::OK);
    let cookie = cookie.unwrap();
    let (s, ..) = send(&app, upload(&cookie)).await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, _, body) = send(&app, get("/api/meetings", &cookie)).await;
    let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows[0]["title"], "Weekly sync");
    assert_eq!(rows[0]["status"], "uploaded");

    // A transcribe job was queued and the audio file was stored.
    let tid = db::tenant_ids(&db::open_control(dir.path()).unwrap())
        .unwrap()
        .remove(0);
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let n: i64 = t
        .query_row(
            "SELECT count(*) FROM jobs WHERE kind='transcribe'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}
