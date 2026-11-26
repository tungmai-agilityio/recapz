use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use recapz_core::db;
use recapz_server::{config::Config, routes};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Option<String>, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
        .find(|c| c.starts_with("recapz_session="));
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

fn req(method: &str, uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .body(Body::empty())
        .unwrap()
}

struct Fixture {
    app: Router,
    control: Arc<Mutex<rusqlite::Connection>>,
    dir: tempfile::TempDir,
    first: String,
    second: String,
    boss: String,
    amy: String,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let control = db::open_control(dir.path()).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    let first = db::tenant_ids(&control).unwrap().remove(0);
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&first],
        )
        .unwrap();
    // The second workspace invites boss as admin; boss is a member of both after signing in.
    let second = db::create_workspace(&control, dir.path(), "Second", "boss@example.com").unwrap();
    for (tid, title) in [(&first, "First meeting"), (&second, "Second meeting")] {
        db::open_tenant(dir.path(), tid)
            .unwrap()
            .execute(
                "INSERT INTO meetings (id, title, platform, status, share) VALUES (?1, ?2, 'upload', 'ready', 'workspace')",
                [format!("m-{title}"), title.to_string()],
            )
            .unwrap();
    }
    let control = Arc::new(Mutex::new(control));
    let cfg = Config {
        data_dir: dir.path().into(),
        dev_login: true,
        ..Config::default()
    };
    let app = routes::router(cfg, control.clone());
    let boss = send(&app, login("boss@example.com")).await.1.unwrap();
    let amy = send(&app, login("amy@example.com")).await.1.unwrap();
    Fixture {
        app,
        control,
        dir,
        first,
        second,
        boss,
        amy,
    }
}

impl Fixture {
    async fn json(&self, who: &str, method: &str, uri: &str) -> (StatusCode, Value) {
        let (s, _, b) = send(&self.app, req(method, uri, who)).await;
        (s, serde_json::from_str(&b).unwrap_or(Value::String(b)))
    }

    async fn titles(&self, who: &str) -> Vec<String> {
        let (s, v) = self.json(who, "GET", "/api/meetings").await;
        assert_eq!(s, StatusCode::OK);
        v.as_array()
            .unwrap()
            .iter()
            .map(|m| m["title"].as_str().unwrap().to_owned())
            .collect()
    }
}

#[tokio::test]
async fn switching_changes_the_meetings_the_session_sees() {
    let f = fixture().await;
    let (s, v) = f.json(&f.boss, "GET", "/api/me").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["workspace"], json!({"id": f.first, "name": "Workspace"}));
    assert_eq!(f.titles(&f.boss).await, ["First meeting"]);

    let (s, v) = f.json(&f.boss, "GET", "/api/workspaces").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        v,
        json!([
            {"id": f.second, "name": "Second", "role": "admin", "current": false},
            {"id": f.first, "name": "Workspace", "role": "admin", "current": true},
        ])
    );

    let (s, _) = f
        .json(
            &f.boss,
            "POST",
            &format!("/api/workspaces/{}/switch", f.second),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(f.titles(&f.boss).await, ["Second meeting"]);
    let (_, v) = f.json(&f.boss, "GET", "/api/me").await;
    assert_eq!(v["workspace"], json!({"id": f.second, "name": "Second"}));
    let (_, v) = f.json(&f.boss, "GET", "/api/workspaces").await;
    assert_eq!(v[0]["current"], true);
    assert_eq!(v[1]["current"], false);

    // The choice is remembered for the next sign-in, and another session keeps its own workspace.
    let used_last: String = f
        .control
        .lock()
        .unwrap()
        .query_row(
            "SELECT tenant_id FROM users WHERE email = 'boss@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(used_last, f.second);
    let other = send(&f.app, login("boss@example.com")).await.1.unwrap();
    assert_eq!(f.titles(&other).await, ["Second meeting"]);
    assert_eq!(f.titles(&f.boss).await, ["Second meeting"]);

    // And back.
    f.json(
        &f.boss,
        "POST",
        &format!("/api/workspaces/{}/switch", f.first),
    )
    .await;
    assert_eq!(f.titles(&f.boss).await, ["First meeting"]);
    assert_eq!(f.titles(&other).await, ["Second meeting"]);
}

#[tokio::test]
async fn no_membership_is_403_and_changes_nothing() {
    let f = fixture().await;
    let uri = format!("/api/workspaces/{}/switch", f.second);
    let (s, v) = f.json(&f.amy, "POST", &uri).await;
    assert_eq!(
        (s, v),
        (
            StatusCode::FORBIDDEN,
            Value::String("you are not a member of this workspace".into())
        )
    );
    let (s, _) = f.json(&f.amy, "POST", "/api/workspaces/nope/switch").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(f.titles(&f.amy).await, ["First meeting"]);
    let (_, v) = f.json(&f.amy, "GET", "/api/workspaces").await;
    assert_eq!(
        v,
        json!([{"id": f.first, "name": "Workspace", "role": "member", "current": true}])
    );

    // Signed out is 401; without the CSRF header the switch is refused.
    let (s, ..) = send(&f.app, Request::post(&uri).body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, ..) = send(
        &f.app,
        Request::post(format!("/api/workspaces/{}/switch", f.second))
            .header(header::COOKIE, format!("{}; recapz_csrf=t", f.boss))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(f.titles(&f.boss).await, ["First meeting"]);
}

#[tokio::test]
async fn listing_accepts_a_pending_invite() {
    let f = fixture().await;
    let third = db::create_workspace(
        &f.control.lock().unwrap(),
        f.dir.path(),
        "Third",
        "amy@example.com",
    )
    .unwrap();
    let (_, v) = f.json(&f.amy, "GET", "/api/workspaces").await;
    assert_eq!(
        v,
        json!([
            {"id": third, "name": "Third", "role": "admin", "current": false},
            {"id": f.first, "name": "Workspace", "role": "member", "current": true},
        ])
    );
    let (s, _) = f
        .json(&f.amy, "POST", &format!("/api/workspaces/{third}/switch"))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, v) = f.json(&f.amy, "GET", "/api/me").await;
    assert_eq!(
        (&v["role"], &v["workspace"]["id"]),
        (&json!("admin"), &json!(third))
    );
}
