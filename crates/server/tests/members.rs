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

fn req(method: &str, uri: &str, cookie: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

struct Fixture {
    app: Router,
    dir: tempfile::TempDir,
    tid: String,
    boss: String,
    amy: String,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let control = db::open_control(dir.path()).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let cfg = Config {
        data_dir: dir.path().into(),
        dev_login: true,
        base_url: "https://recapz.example".into(),
        ..Config::default()
    };
    let app = routes::router(cfg, Arc::new(Mutex::new(control)));
    let boss = send(&app, login("boss@example.com")).await.1.unwrap();
    let amy = send(&app, login("amy@example.com")).await.1.unwrap();
    Fixture {
        app,
        dir,
        tid,
        boss,
        amy,
    }
}

impl Fixture {
    async fn call(&self, who: &str, method: &str, uri: &str, body: &str) -> (StatusCode, String) {
        let (s, _, b) = send(&self.app, req(method, uri, who, body)).await;
        (s, b)
    }

    /// Id of the member with this email, read from the list.
    async fn id_of(&self, email: &str) -> String {
        let (_, b) = self.call(&self.boss, "GET", "/api/members", "").await;
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        v["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["email"] == email)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn tenant_row(&self, email: &str) -> (String, String) {
        db::open_tenant(self.dir.path(), &self.tid)
            .unwrap()
            .query_row(
                "SELECT role, status FROM members WHERE email = ?1",
                [email],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }
}

#[tokio::test]
async fn a_member_is_refused_on_every_route_and_csrf_applies() {
    let f = fixture().await;
    let id = f.id_of("boss@example.com").await;
    let routes = [
        ("GET", "/api/members".to_owned(), ""),
        (
            "POST",
            "/api/members/invites".to_owned(),
            r#"{"email":"x@example.com","role":"member"}"#,
        ),
        (
            "DELETE",
            "/api/members/invites/x@example.com".to_owned(),
            "",
        ),
        (
            "PATCH",
            format!("/api/members/{id}"),
            r#"{"role":"member"}"#,
        ),
        ("DELETE", format!("/api/members/{id}"), ""),
    ];
    assert_eq!(routes.len(), 5);
    for (m, uri, body) in &routes {
        let (s, b) = f.call(&f.amy, m, uri, body).await;
        assert_eq!(
            (s, b.as_str()),
            (StatusCode::FORBIDDEN, "only an admin can manage members"),
            "{m} {uri}"
        );
    }
    let (_, b) = f.call(&f.boss, "GET", "/api/members", "").await;
    assert!(b.contains("boss@example.com"), "admin is unaffected: {b}");
    let (s, _, _) = send(&f.app, {
        // Same call as an admin, without the CSRF header.
        Request::post("/api/members/invites")
            .header(header::COOKIE, format!("{}; recapz_csrf=t", f.boss))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"email":"x@example.com","role":"member"}"#))
            .unwrap()
    })
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, b) = f.call(&f.boss, "GET", "/api/members", "").await;
    assert!(!b.contains("x@example.com"), "no invite without CSRF: {b}");
}

#[tokio::test]
async fn admin_lists_invites_changes_roles_and_removes() {
    let f = fixture().await;
    let (s, b) = f.call(&f.boss, "GET", "/api/members", "").await;
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(s, StatusCode::OK);
    let emails: Vec<_> = v["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["email"].as_str().unwrap(), m["role"].as_str().unwrap()))
        .collect();
    assert_eq!(
        emails,
        [("amy@example.com", "member"), ("boss@example.com", "admin")]
    );
    assert_eq!(v["invites"].as_array().unwrap().len(), 0);

    // Invite: returns the link, lists as pending, refuses bad input and existing members.
    let inv = |e: &str, r: &str| format!(r#"{{"email":"{e}","role":"{r}"}}"#);
    let (s, b) = f
        .call(
            &f.boss,
            "POST",
            "/api/members/invites",
            &inv("Bob@Example.com", "member"),
        )
        .await;
    assert_eq!(
        (s, b.as_str()),
        (StatusCode::CREATED, r#"{"link":"https://recapz.example/"}"#)
    );
    let (_, b) = f.call(&f.boss, "GET", "/api/members", "").await;
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["invites"][0]["email"], "bob@example.com");
    assert_eq!(v["invites"].as_array().unwrap().len(), 1);
    for (body, want) in [
        (inv("amy@example.com", "member"), StatusCode::CONFLICT),
        (inv("nobody", "member"), StatusCode::BAD_REQUEST),
        (inv("c@example.com", "owner"), StatusCode::BAD_REQUEST),
    ] {
        let (s, _) = f.call(&f.boss, "POST", "/api/members/invites", &body).await;
        assert_eq!(s, want, "{body}");
    }
    let (s, _) = f
        .call(
            &f.boss,
            "DELETE",
            "/api/members/invites/bob@example.com",
            "",
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = f
        .call(
            &f.boss,
            "DELETE",
            "/api/members/invites/bob@example.com",
            "",
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Role change shows inside the tenant on the person's next request.
    let amy_id = f.id_of("amy@example.com").await;
    let uri = format!("/api/members/{amy_id}");
    f.call(&f.amy, "GET", "/api/meetings", "").await;
    assert_eq!(f.tenant_row("amy@example.com").0, "member");
    let (s, _) = f.call(&f.boss, "PATCH", &uri, r#"{"role":"admin"}"#).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(f.tenant_row("amy@example.com").0, "member");
    f.call(&f.amy, "GET", "/api/meetings", "").await;
    assert_eq!(
        f.tenant_row("amy@example.com"),
        ("admin".into(), "active".into())
    );
    let (s, _) = f.call(&f.boss, "PATCH", &uri, r#"{"role":"x"}"#).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = f
        .call(
            &f.boss,
            "PATCH",
            "/api/members/nope",
            r#"{"role":"member"}"#,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Remove: the person is signed out, the row stays removed, and a new invite revives it.
    let (s, _) = f.call(&f.boss, "DELETE", &uri, "").await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = f.call(&f.amy, "GET", "/api/meetings", "").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        f.tenant_row("amy@example.com"),
        ("admin".into(), "removed".into())
    );
    let (s, _) = f.call(&f.boss, "DELETE", &uri, "").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    f.call(
        &f.boss,
        "POST",
        "/api/members/invites",
        &inv("amy@example.com", "member"),
    )
    .await;
    let amy = send(&f.app, login("amy@example.com")).await.1.unwrap();
    let (s, _) = f.call(&amy, "GET", "/api/meetings", "").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        f.tenant_row("amy@example.com"),
        ("member".into(), "active".into())
    );
}

#[tokio::test]
async fn last_admin_and_self_removal_are_refused() {
    let f = fixture().await;
    let boss_id = f.id_of("boss@example.com").await;
    let uri = format!("/api/members/{boss_id}");
    let (s, b) = f.call(&f.boss, "DELETE", &uri, "").await;
    assert_eq!(
        (s, b.as_str()),
        (StatusCode::FORBIDDEN, "you cannot remove yourself")
    );
    let (s, b) = f.call(&f.boss, "PATCH", &uri, r#"{"role":"member"}"#).await;
    assert_eq!(
        (s, b.as_str()),
        (StatusCode::CONFLICT, "a workspace needs at least one admin")
    );
    // Still an admin after both refusals.
    let (s, _) = f.call(&f.boss, "GET", "/api/members", "").await;
    assert_eq!(s, StatusCode::OK);

    // With a second admin, the first can be demoted, and the last one cannot be removed by a peer.
    let amy_id = f.id_of("amy@example.com").await;
    let (s, _) = f
        .call(
            &f.boss,
            "PATCH",
            &format!("/api/members/{amy_id}"),
            r#"{"role":"admin"}"#,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = f.call(&f.boss, "PATCH", &uri, r#"{"role":"member"}"#).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, b) = f
        .call(
            &f.amy,
            "PATCH",
            &format!("/api/members/{amy_id}"),
            r#"{"role":"member"}"#,
        )
        .await;
    assert_eq!(
        (s, b.as_str()),
        (StatusCode::CONFLICT, "a workspace needs at least one admin")
    );
}
