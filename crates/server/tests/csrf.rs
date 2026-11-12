use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
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

/// Signs in and returns (all Set-Cookie pairs joined as a Cookie header, csrf token).
async fn sign_in(app: &Router) -> (String, String) {
    let req = Request::post("/auth/dev-login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"email":"boss@example.com"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let pairs: Vec<String> = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
        .collect();
    assert_eq!(pairs.len(), 2, "session and csrf cookies: {pairs:?}");
    let token = pairs
        .iter()
        .find_map(|p| p.strip_prefix("recapz_csrf="))
        .expect("csrf cookie")
        .to_owned();
    (pairs.join("; "), token)
}

fn logout(cookie: &str, token: Option<&str>, origin: Option<&str>) -> Request<Body> {
    let mut r = Request::post("/auth/logout").header(header::COOKIE, cookie);
    if let Some(t) = token {
        r = r.header("x-csrf-token", t);
    }
    if let Some(o) = origin {
        r = r.header(header::ORIGIN, o);
    }
    r.body(Body::empty()).unwrap()
}

async fn status(app: &Router, req: Request<Body>) -> StatusCode {
    app.clone().oneshot(req).await.unwrap().status()
}

fn me(cookie: &str) -> Request<Body> {
    Request::get("/api/me")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn state_changing_requests_need_the_matching_token_and_site_origin() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (cookie, token) = sign_in(&app).await;

    for bad in [
        logout(&cookie, None, None),
        logout(&cookie, Some("wrong"), None),
        logout(&cookie, Some(&token), Some("https://evil.example")),
    ] {
        assert_eq!(status(&app, bad).await, StatusCode::FORBIDDEN);
        assert_eq!(
            status(&app, me(&cookie)).await,
            StatusCode::OK,
            "session survives"
        );
    }

    // The same site's origin, as a browser sends it, passes.
    let ok = logout(&cookie, Some(&token), Some("http://localhost:8080"));
    assert_eq!(status(&app, ok).await, StatusCode::OK);
    assert_eq!(status(&app, me(&cookie)).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_request_without_a_session_still_gets_401_not_403() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let req = Request::post("/api/ask")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"question":"x"}"#))
        .unwrap();
    assert_eq!(status(&app, req).await, StatusCode::UNAUTHORIZED);
}
