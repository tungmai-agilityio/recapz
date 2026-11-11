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
