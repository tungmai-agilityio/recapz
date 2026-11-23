//! Desktop sign-in: Google in the system browser, then a one-time code swapped for the session cookie.
use axum::{
    body::Body,
    extract::Form,
    http::{header, Request, StatusCode},
    routing::{get, post},
    Json, Router,
};
use recapz_core::db;
use recapz_server::{
    config::{Config, Google},
    routes,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

const ORIGIN: &str = "http://localhost:8080";

/// Mock Google: code `boss` is boss@example.com (a member), anything else is stranger@example.com.
async fn mock_google() -> String {
    let app = Router::new()
        .route(
            "/token",
            post(|Form(f): Form<HashMap<String, String>>| async move {
                let at = if f["code"] == "boss" {
                    "at-boss"
                } else {
                    "at-stranger"
                };
                Json(serde_json::json!({ "access_token": at }))
            }),
        )
        .route(
            "/userinfo",
            get(|h: axum::http::HeaderMap| async move {
                let email = if h[header::AUTHORIZATION] == "Bearer at-boss" {
                    "boss@example.com"
                } else {
                    "stranger@example.com"
                };
                Json(serde_json::json!({ "email": email, "email_verified": true }))
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    base
}

async fn app(dir: &std::path::Path) -> Router {
    let base = mock_google().await;
    let cfg = Config {
        data_dir: dir.into(),
        base_url: ORIGIN.into(),
        google: Google {
            client_id: "cid".into(),
            client_secret: "csecret".into(),
            auth_url: format!("{base}/auth"),
            token_url: format!("{base}/token"),
            userinfo_url: format!("{base}/userinfo"),
            ..Google::default()
        },
        ..Config::default()
    };
    let control = db::open_control(dir).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    routes::router(cfg, Arc::new(Mutex::new(control)))
}

struct Reply {
    status: StatusCode,
    location: Option<String>,
    cookies: Vec<String>,
}

async fn send(app: &Router, req: Request<Body>) -> Reply {
    let res = app.clone().oneshot(req).await.unwrap();
    Reply {
        status: res.status(),
        location: res
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().into()),
        cookies: res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
            .collect(),
    }
}

/// Runs /auth/desktop then the Google callback as `google_code`; returns the callback reply.
async fn browser_sign_in(app: &Router, google_code: &str) -> Reply {
    let start = send(
        app,
        Request::get("/auth/desktop").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(start.status, StatusCode::SEE_OTHER);
    assert!(start.location.unwrap().contains("/auth?client_id=cid"));
    let cookie = start.cookies.join("; ");
    let state = start
        .cookies
        .iter()
        .find_map(|c| c.strip_prefix("recapz_oauth_state="))
        .unwrap();
    send(
        app,
        Request::get(format!("/auth/callback?code={google_code}&state={state}"))
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

fn code_of(r: &Reply) -> String {
    let loc = r.location.as_deref().unwrap();
    loc.strip_prefix("recapz://signed-in?code=")
        .unwrap_or_else(|| panic!("{loc}"))
        .to_owned()
}

fn exchange(code: &str, origin: Option<&str>, cookie: Option<&str>) -> Request<Body> {
    let mut r =
        Request::post("/auth/desktop/exchange").header(header::CONTENT_TYPE, "application/json");
    if let Some(o) = origin {
        r = r.header(header::ORIGIN, o);
    }
    if let Some(c) = cookie {
        r = r.header(header::COOKIE, c);
    }
    r.body(Body::from(format!(r#"{{"code":"{code}"}}"#)))
        .unwrap()
}

#[tokio::test]
async fn member_signs_in_through_the_browser_and_exchanges_the_code_for_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path()).await;

    let cb = browser_sign_in(&app, "boss").await;
    assert_eq!(cb.status, StatusCode::SEE_OTHER);
    let code = code_of(&cb);
    assert_eq!(code.len(), 64, "two UUIDs without dashes");
    assert!(
        !cb.cookies.iter().any(|c| c.starts_with("recapz_session=")),
        "the browser itself gets no session: {:?}",
        cb.cookies
    );

    let ex = send(&app, exchange(&code, Some(ORIGIN), None)).await;
    assert_eq!(ex.status, StatusCode::OK);
    assert_eq!(ex.cookies.len(), 2, "session and csrf: {:?}", ex.cookies);
    let me = send(
        &app,
        Request::get("/api/me")
            .header(header::COOKIE, ex.cookies.join("; "))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(me.status, StatusCode::OK);
}

#[tokio::test]
async fn a_code_works_once() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path()).await;
    let code = code_of(&browser_sign_in(&app, "boss").await);

    assert_eq!(
        send(&app, exchange(&code, Some(ORIGIN), None)).await.status,
        StatusCode::OK
    );
    let again = send(&app, exchange(&code, Some(ORIGIN), None)).await;
    assert_eq!(again.status, StatusCode::FORBIDDEN);
    assert!(again.cookies.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_code_expires_after_sixty_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path()).await;
    let code = code_of(&browser_sign_in(&app, "boss").await);
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    assert_eq!(
        send(&app, exchange(&code, Some(ORIGIN), None)).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test(start_paused = true)]
async fn a_code_still_works_just_inside_sixty_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path()).await;
    let code = code_of(&browser_sign_in(&app, "boss").await);
    tokio::time::advance(std::time::Duration::from_secs(59)).await;
    assert_eq!(
        send(&app, exchange(&code, Some(ORIGIN), None)).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn an_unknown_code_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path()).await;
    assert_eq!(
        send(&app, exchange("nope", Some(ORIGIN), None))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn someone_who_is_not_a_member_or_invited_gets_no_code() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path()).await;
    let cb = browser_sign_in(&app, "stranger").await;
    assert_eq!(cb.status, StatusCode::FORBIDDEN);
    assert!(cb.location.is_none());
}

#[tokio::test]
async fn the_exchange_needs_this_sites_origin_and_the_csrf_token_of_a_stale_session() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path()).await;
    let code = code_of(&browser_sign_in(&app, "boss").await);

    // Wrong or missing Origin: refused, and the code stays usable.
    for origin in [Some("https://evil.example"), None] {
        assert_eq!(
            send(&app, exchange(&code, origin, None)).await.status,
            StatusCode::FORBIDDEN
        );
    }
    // A session cookie without a matching x-csrf-token is stopped by the CSRF guard.
    let stale = "recapz_session=old; recapz_csrf=t";
    assert_eq!(
        send(&app, exchange(&code, Some(ORIGIN), Some(stale)))
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    assert_eq!(
        send(&app, exchange(&code, Some(ORIGIN), None)).await.status,
        StatusCode::OK
    );
}
