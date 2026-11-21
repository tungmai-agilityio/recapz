//! Connect and disconnect Google Calendar against a local mock of Google's OAuth endpoints.
use axum::{
    body::Body,
    extract::{Form, State},
    http::{header, HeaderMap, Request, StatusCode},
    routing::{get, post},
    Json, Router,
};
use recapz_core::{
    db,
    google_tokens::{self, SecretKey},
};
use recapz_server::{
    config::{Config, Google},
    routes,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const CAL: &str = "https://www.googleapis.com/auth/calendar.events.readonly";

type Revoked = Arc<Mutex<Vec<String>>>;

/// Codes: `good` grants calendar for boss@, `unticked` grants no calendar, `other` is another account.
async fn mock_google() -> (String, Revoked) {
    let revoked: Revoked = Default::default();
    let app = Router::new()
        .route(
            "/token",
            post(|Form(f): Form<HashMap<String, String>>| async move {
                let (scope, access) = match f.get("code").map(String::as_str) {
                    Some("unticked") => ("openid email".to_string(), "at-boss"),
                    Some("other") => (format!("openid email {CAL}"), "at-other"),
                    _ => (format!("openid email {CAL}"), "at-boss"),
                };
                Json(serde_json::json!({"access_token": access, "refresh_token": "rt-1", "scope": scope}))
            }),
        )
        .route(
            "/userinfo",
            get(|h: HeaderMap| async move {
                let email = if h[header::AUTHORIZATION] == "Bearer at-other" { "other@example.com" } else { "boss@example.com" };
                Json(serde_json::json!({"email": email, "email_verified": true}))
            }),
        )
        .route(
            "/revoke",
            post(|State(r): State<Revoked>, Form(f): Form<HashMap<String, String>>| async move {
                r.lock().unwrap().push(f["token"].clone());
            }),
        )
        .with_state(revoked.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, revoked)
}

fn app(dir: &std::path::Path, google: Google, secret_key: &str) -> Router {
    let cfg = Config {
        data_dir: dir.into(),
        dev_login: true,
        google,
        secret_key: secret_key.into(),
        ..Config::default()
    };
    let control = db::open_control(dir).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    routes::router(cfg, Arc::new(Mutex::new(control)))
}

fn google(base: &str) -> Google {
    Google {
        client_id: "cid".into(),
        client_secret: "csecret".into(),
        auth_url: format!("{base}/auth"),
        token_url: format!("{base}/token"),
        userinfo_url: format!("{base}/userinfo"),
        revoke_url: format!("{base}/revoke"),
        ..Google::default()
    }
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

async fn sign_in(app: &Router) -> String {
    let req = Request::post("/auth/dev-login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"email":"boss@example.com"}"#))
        .unwrap();
    let r = send(app, req).await;
    r.cookies
        .into_iter()
        .find(|c| c.starts_with("recapz_session="))
        .unwrap()
}

/// Starts the flow and returns the state value the callback must echo, plus its cookie.
async fn start(app: &Router, session: &str) -> (String, String, Reply) {
    let r = send(
        app,
        Request::get("/auth/calendar")
            .header(header::COOKIE, session)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let state_cookie = r
        .cookies
        .iter()
        .find(|c| c.starts_with("recapz_cal_state="))
        .cloned()
        .unwrap();
    let state = state_cookie.split('=').nth(1).unwrap().to_owned();
    (state, state_cookie, r)
}

async fn callback(app: &Router, session: &str, code: &str) -> Reply {
    let (state, state_cookie, _) = start(app, session).await;
    let req = Request::get(format!("/auth/calendar/callback?code={code}&state={state}"))
        .header(header::COOKIE, format!("{session}; {state_cookie}"))
        .body(Body::empty())
        .unwrap();
    send(app, req).await
}

fn stored(dir: &std::path::Path) -> Option<(String, String)> {
    let tenant = db::tenant_ids(&db::open_control(dir).unwrap())
        .unwrap()
        .remove(0);
    let c = db::open_tenant(dir, &tenant).unwrap();
    let id: String = c
        .query_row("SELECT member_id FROM google_tokens LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap_or_default();
    if id.is_empty() {
        return None; // no row at all
    }
    google_tokens::load(&c, &SecretKey::from_hex(KEY).unwrap(), &id).unwrap()
}

async fn connected(app: &Router, session: &str) -> String {
    let r = app
        .clone()
        .oneshot(
            Request::get("/api/calendar")
                .header(header::COOKIE, session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    use http_body_util::BodyExt;
    String::from_utf8(r.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
}

#[tokio::test]
async fn connecting_asks_for_offline_calendar_access_and_stores_the_encrypted_token() {
    let dir = tempfile::tempdir().unwrap();
    let (base, _) = mock_google().await;
    let app = app(dir.path(), google(&base), KEY);
    let session = sign_in(&app).await;
    let (_, _, started) = start(&app, &session).await;
    let to = started.location.unwrap();
    for part in [
        "client_id=cid",
        "access_type=offline",
        "prompt=consent",
        "include_granted_scopes=true",
        "scope=openid%20email%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcalendar.events.readonly",
    ] {
        assert!(to.contains(part), "{part} in {to}");
    }
    assert!(to.starts_with(&format!("{base}/auth?")));

    assert_eq!(
        connected(&app, &session).await,
        r#"{"connected":false,"needs_reconnect":false}"#
    );
    let r = callback(&app, &session, "good").await;
    assert_eq!(r.location.as_deref(), Some("/recording?calendar=connected"));
    assert_eq!(
        stored(dir.path()),
        Some(("rt-1".into(), format!("openid email {CAL}")))
    );
    assert_eq!(
        connected(&app, &session).await,
        r#"{"connected":true,"needs_reconnect":false}"#
    );
}

#[tokio::test]
async fn nothing_is_stored_when_calendar_is_unticked_or_the_account_differs() {
    let dir = tempfile::tempdir().unwrap();
    let (base, revoked) = mock_google().await;
    let app = app(dir.path(), google(&base), KEY);
    let session = sign_in(&app).await;
    for (code, outcome) in [("unticked", "scope_missing"), ("other", "wrong_account")] {
        let r = callback(&app, &session, code).await;
        assert_eq!(
            r.location,
            Some(format!("/recording?calendar={outcome}")),
            "{code}"
        );
        assert_eq!(stored(dir.path()), None, "{code}");
    }
    // Both grants Google issued but we refused are cancelled at Google.
    assert_eq!(*revoked.lock().unwrap(), vec!["rt-1".to_string(); 2]);
    // Declined or refused by Google: no code comes back.
    let (state, state_cookie, _) = start(&app, &session).await;
    let req = Request::get(format!(
        "/auth/calendar/callback?error=access_denied&state={state}"
    ))
    .header(header::COOKIE, format!("{session}; {state_cookie}"))
    .body(Body::empty())
    .unwrap();
    assert_eq!(
        send(&app, req).await.location.as_deref(),
        Some("/recording?calendar=denied")
    );
    assert_eq!(stored(dir.path()), None);
}

#[tokio::test]
async fn a_wrong_state_or_missing_session_or_key_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (base, _) = mock_google().await;
    let app = app(dir.path(), google(&base), KEY);
    let session = sign_in(&app).await;
    let (_, state_cookie, _) = start(&app, &session).await;
    let req = Request::get("/auth/calendar/callback?code=good&state=forged")
        .header(header::COOKIE, format!("{session}; {state_cookie}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, req).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        send(
            &app,
            Request::get("/auth/calendar").body(Body::empty()).unwrap()
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );

    let dir2 = tempfile::tempdir().unwrap();
    let no_key = self::app(dir2.path(), google(&base), "");
    let s2 = sign_in(&no_key).await;
    let r = send(
        &no_key,
        Request::get("/auth/calendar")
            .header(header::COOKIE, &s2)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn disconnect_revokes_at_google_and_hard_deletes_the_token() {
    let dir = tempfile::tempdir().unwrap();
    let (base, revoked) = mock_google().await;
    let app = app(dir.path(), google(&base), KEY);
    let session = sign_in(&app).await;
    callback(&app, &session, "good").await;
    assert!(stored(dir.path()).is_some());

    let del = |s: &str| {
        Request::delete("/api/calendar")
            .header(header::COOKIE, format!("{s}; recapz_csrf=t"))
            .header("x-csrf-token", "t")
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(&app, del(&session)).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(stored(dir.path()), None);
    assert_eq!(*revoked.lock().unwrap(), vec!["rt-1".to_string()]);
    assert_eq!(
        connected(&app, &session).await,
        r#"{"connected":false,"needs_reconnect":false}"#
    );
    // Disconnecting again is harmless and calls Google no second time.
    assert_eq!(
        send(&app, del(&session)).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(revoked.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn disconnect_deletes_the_token_even_when_google_cannot_be_reached() {
    let dir = tempfile::tempdir().unwrap();
    let (base, _) = mock_google().await;
    let mut g = google(&base);
    let app_ok = app(dir.path(), g.clone(), KEY);
    let session = sign_in(&app_ok).await;
    callback(&app_ok, &session, "good").await;
    g.revoke_url = "http://127.0.0.1:1/revoke".into(); // nothing listens there
    let app = self::app(dir.path(), g, KEY);
    let req = Request::delete("/api/calendar")
        .header(header::COOKIE, format!("{session}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, req).await.status, StatusCode::NO_CONTENT);
    assert_eq!(stored(dir.path()), None);
}

#[tokio::test]
async fn connecting_fetches_the_calendar_at_once_instead_of_waiting_for_the_next_tick() {
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let dir = tempfile::tempdir().unwrap();
    let (base, _) = mock_google().await;
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let events = Router::new().route(
        "/events",
        get(move |hd: HeaderMap| async move {
            assert_eq!(hd[header::AUTHORIZATION], "Bearer at-boss");
            h.fetch_add(1, SeqCst);
            Json(serde_json::json!({"items": []}))
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let events_url = format!("http://{}/events", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, events).await.unwrap() });
    let app = app(
        dir.path(),
        Google {
            events_url,
            ..google(&base)
        },
        KEY,
    );
    let session = sign_in(&app).await;
    assert_eq!(hits.load(SeqCst), 0);
    callback(&app, &session, "good").await;
    for _ in 0..100 {
        if hits.load(SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(hits.load(SeqCst), 1);
}
