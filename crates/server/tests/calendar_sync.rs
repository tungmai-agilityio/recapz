//! Calendar sync with stored tokens, against a local mock of Google's token and events endpoints.
use axum::{
    extract::{Form, Query, State},
    http::{header, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use recapz_core::{
    calendar, db,
    google_tokens::{self, SecretKey},
};
use recapz_server::{
    calendar_sync::{sync_member, Outcome},
    config::Google,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const NOW: i64 = 1_800_000_000_000; // inside the sync window of every event below

#[derive(Clone, Default)]
struct Mock {
    refreshes: Arc<AtomicUsize>,
}

fn ev(id: &str, title: &str, minute: i64) -> serde_json::Value {
    let t = |m: i64| {
        let secs = (NOW / 1000) + m * 60;
        time::OffsetDateTime::from_unix_timestamp(secs)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    };
    serde_json::json!({"id": id, "summary": title,
        "start": {"dateTime": t(minute)}, "end": {"dateTime": t(minute + 30)},
        "hangoutLink": "https://meet.google.com/abc-defg-hij"})
}

async fn mock_google(events_mode: &'static str) -> (Google, Mock) {
    let mock = Mock::default();
    let app = Router::new()
        .route(
            "/token",
            post(|State(m): State<Mock>, Form(f): Form<HashMap<String, String>>| async move {
                m.refreshes.fetch_add(1, Ordering::SeqCst);
                assert_eq!(f["grant_type"], "refresh_token");
                if f["refresh_token"] == "rt-revoked" {
                    return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "invalid_grant"})));
                }
                (StatusCode::OK, Json(serde_json::json!({"access_token": "at-1", "expires_in": 3600})))
            }),
        )
        .route(
            "/events",
            get(move |h: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                if h[header::AUTHORIZATION] != "Bearer at-1" {
                    return (StatusCode::UNAUTHORIZED, String::new());
                }
                match (events_mode, q.get("pageToken").map(String::as_str)) {
                    ("down", _) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
                    ("huge", _) => (StatusCode::OK, " ".repeat(5 * 1024 * 1024)),
                    (_, None) => (StatusCode::OK, serde_json::json!({"items": [ev("e1", "Standup", 10)], "nextPageToken": "p2"}).to_string()),
                    (_, Some("p2")) => (StatusCode::OK, serde_json::json!({"items": [ev("e2", "Review", 60)]}).to_string()),
                    _ => (StatusCode::BAD_REQUEST, String::new()),
                }
            }),
        )
        .with_state(mock.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let g = Google {
        client_id: "cid".into(),
        client_secret: "csecret".into(),
        token_url: format!("{base}/token"),
        events_url: format!("{base}/events"),
        ..Google::default()
    };
    (g, mock)
}

fn setup(refresh: Option<&str>) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let control = db::open_control(dir.path()).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    let tenant = db::tenant_ids(&control).unwrap().remove(0);
    let c = db::open_tenant(dir.path(), &tenant).unwrap();
    c.execute(
        "INSERT INTO members (id, email, role) VALUES ('u1', 'a@example.com', 'member')",
        [],
    )
    .unwrap();
    if let Some(r) = refresh {
        google_tokens::save(&c, &SecretKey::from_hex(KEY).unwrap(), "u1", r, "s").unwrap();
    }
    (dir, tenant)
}

async fn run(dir: &tempfile::TempDir, tenant: &str, g: &Google) -> anyhow::Result<Outcome> {
    sync_member(
        dir.path(),
        tenant,
        "u1",
        g,
        SecretKey::from_hex(KEY).unwrap(),
        NOW,
    )
    .await
}

fn titles(dir: &tempfile::TempDir, tenant: &str) -> Vec<String> {
    let c = db::open_tenant(dir.path(), tenant).unwrap();
    calendar::upcoming(&c, "u1", NOW, 50)
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect()
}

#[tokio::test]
async fn sync_refreshes_the_token_follows_pages_and_stores_events_without_auto_join() {
    let (dir, tenant) = setup(Some("rt-good"));
    let (g, mock) = mock_google("ok").await;
    assert_eq!(run(&dir, &tenant, &g).await.unwrap(), Outcome::Synced(2));
    assert_eq!(titles(&dir, &tenant), vec!["Standup", "Review"]);
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 1);
    let c = db::open_tenant(dir.path(), &tenant).unwrap();
    let auto: Vec<bool> = calendar::upcoming(&c, "u1", NOW, 50)
        .unwrap()
        .into_iter()
        .map(|e| e.auto_join)
        .collect();
    assert_eq!(auto, vec![false, false], "the default join rule is manual");
    // A second run updates in place and does not duplicate.
    assert_eq!(run(&dir, &tenant, &g).await.unwrap(), Outcome::Synced(2));
    assert_eq!(titles(&dir, &tenant).len(), 2);
}

#[tokio::test]
async fn invalid_grant_deletes_the_token_asks_to_reconnect_and_keeps_synced_events() {
    let (dir, tenant) = setup(Some("rt-good"));
    let (g, _) = mock_google("ok").await;
    run(&dir, &tenant, &g).await.unwrap();
    let c = db::open_tenant(dir.path(), &tenant).unwrap();
    google_tokens::save(
        &c,
        &SecretKey::from_hex(KEY).unwrap(),
        "u1",
        "rt-revoked",
        "s",
    )
    .unwrap();

    assert_eq!(run(&dir, &tenant, &g).await.unwrap(), Outcome::Disconnected);
    assert_eq!(
        google_tokens::load(&c, &SecretKey::from_hex(KEY).unwrap(), "u1").unwrap(),
        None
    );
    assert!(google_tokens::needs_reconnect(&c, "u1").unwrap());
    assert_eq!(
        titles(&dir, &tenant),
        vec!["Standup", "Review"],
        "events already synced stay"
    );
    assert_eq!(run(&dir, &tenant, &g).await.unwrap(), Outcome::NotConnected);
}

#[tokio::test]
async fn other_failures_keep_the_token_and_the_events() {
    for mode in ["down", "huge"] {
        let (dir, tenant) = setup(Some("rt-good"));
        let (g, _) = mock_google("ok").await;
        run(&dir, &tenant, &g).await.unwrap();
        let (bad, _) = mock_google(mode).await;
        assert!(run(&dir, &tenant, &bad).await.is_err(), "{mode}");
        let c = db::open_tenant(dir.path(), &tenant).unwrap();
        assert!(
            google_tokens::load(&c, &SecretKey::from_hex(KEY).unwrap(), "u1")
                .unwrap()
                .is_some(),
            "{mode}"
        );
        assert!(!google_tokens::needs_reconnect(&c, "u1").unwrap(), "{mode}");
        assert_eq!(titles(&dir, &tenant).len(), 2, "{mode}");
    }
}

#[tokio::test]
async fn a_member_without_a_token_is_not_synced() {
    let (dir, tenant) = setup(None);
    let (g, mock) = mock_google("ok").await;
    assert_eq!(run(&dir, &tenant, &g).await.unwrap(), Outcome::NotConnected);
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 0);
}
