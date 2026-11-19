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
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    routes::router(cfg, Arc::new(Mutex::new(control)))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Option<String>, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
        .find(|c| c.starts_with("recapz_session="));
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, cookie, String::from_utf8_lossy(&body).into())
}

async fn login(app: &Router, email: &str) -> String {
    let (s, c, _) = send(
        app,
        Request::post("/auth/dev-login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(r#"{{"email":"{email}"}}"#)))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    c.unwrap()
}

fn req(method: &str, uri: &str, cookie: &str, body: Body, ct: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, ct)
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .body(body)
        .unwrap()
}

fn upload(cookie: &str, name: &str, content: &[u8]) -> Request<Body> {
    let mut b = format!(
        "--XX\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n\r\n"
    )
    .into_bytes();
    b.extend_from_slice(content);
    b.extend_from_slice(b"\r\n--XX--\r\n");
    req(
        "POST",
        "/api/documents",
        cookie,
        Body::from(b),
        "multipart/form-data; boundary=XX",
    )
}

fn get(cookie: &str) -> Request<Body> {
    req("GET", "/api/documents", cookie, Body::empty(), "text/plain")
}

fn patch(cookie: &str, id: &str, on: bool) -> Request<Body> {
    req(
        "PATCH",
        &format!("/api/documents/{id}"),
        cookie,
        Body::from(format!(r#"{{"llm_ok":{on}}}"#)),
        "application/json",
    )
}

fn delete(cookie: &str, id: &str) -> Request<Body> {
    req(
        "DELETE",
        &format!("/api/documents/{id}"),
        cookie,
        Body::empty(),
        "text/plain",
    )
}

fn id_of(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn files(dir: &std::path::Path) -> usize {
    let control = db::open_control(dir).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    match std::fs::read_dir(db::tenant_dir(dir, &tid).join("documents")) {
        Ok(d) => d.count(),
        Err(_) => 0,
    }
}

#[tokio::test]
async fn upload_lists_with_toggle_off_and_indexing_status() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let boss = login(&app, "boss@example.com").await;
    let (s, _, b) = send(&app, upload(&boss, "notes.md", b"# hello")).await;
    assert_eq!(s, StatusCode::CREATED);
    let id = id_of(&b);
    let (s, _, b) = send(&app, get(&boss)).await;
    assert_eq!(s, StatusCode::OK);
    let rows: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], id);
    assert_eq!(rows[0]["name"], "notes.md");
    assert_eq!(rows[0]["llm_ok"], false);
    assert_eq!(rows[0]["status"], "indexing");
}

#[tokio::test]
async fn only_markdown_and_txt_utf8_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let boss = login(&app, "boss@example.com").await;
    for (name, body) in [("a.pdf", &b"x"[..]), ("a.txt", &[0xff, 0xfe][..])] {
        let (s, _, _) = send(&app, upload(&boss, name, body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{name}");
    }
    assert_eq!(files(dir.path()), 0);
    assert_eq!(send(&app, get(&boss)).await.2, "[]");
}

#[tokio::test]
async fn a_file_over_20_mb_is_refused_and_nothing_is_stored() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let boss = login(&app, "boss@example.com").await;
    let big = vec![b'a'; 21 * 1024 * 1024];
    let (s, _, _) = send(&app, upload(&boss, "big.txt", &big)).await;
    assert!(
        s == StatusCode::BAD_REQUEST || s == StatusCode::PAYLOAD_TOO_LARGE,
        "{s}"
    );
    assert_eq!(files(dir.path()), 0);
    assert_eq!(send(&app, get(&boss)).await.2, "[]");
    // Just under the limit still works, so the limit is the document one, not a smaller default.
    let ok = vec![b'a'; 20 * 1024 * 1024 - 1];
    assert_eq!(
        send(&app, upload(&boss, "ok.txt", &ok)).await.0,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn a_member_at_500_mb_cannot_add_more() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let boss = login(&app, "boss@example.com").await;
    let (_, _, b) = send(&app, upload(&boss, "a.txt", b"x")).await;
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    db::open_tenant(dir.path(), &tid)
        .unwrap()
        .execute(
            "UPDATE documents SET size_bytes=?2 WHERE id=?1",
            rusqlite::params![id_of(&b), 500i64 * 1024 * 1024],
        )
        .unwrap();
    let (s, _, _) = send(&app, upload(&boss, "b.txt", b"y")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(files(dir.path()), 1);
}

#[tokio::test]
async fn another_member_cannot_list_patch_or_delete_my_document() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let boss = login(&app, "boss@example.com").await;
    let amy = login(&app, "amy@example.com").await;
    let (_, _, b) = send(&app, upload(&boss, "mine.md", b"secret")).await;
    let id = id_of(&b);
    assert_eq!(send(&app, get(&amy)).await.2, "[]");
    assert_eq!(
        send(&app, patch(&amy, &id, true)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(send(&app, delete(&amy, &id)).await.0, StatusCode::NOT_FOUND);
    // Untouched: still listed for the owner, toggle still off, file still on disk.
    let rows: serde_json::Value = serde_json::from_str(&send(&app, get(&boss)).await.2).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["llm_ok"], false);
    assert_eq!(files(dir.path()), 1);
}

#[tokio::test]
async fn patch_toggles_and_delete_removes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let boss = login(&app, "boss@example.com").await;
    let (_, _, b) = send(&app, upload(&boss, "mine.md", b"secret")).await;
    let id = id_of(&b);
    assert_eq!(
        send(&app, patch(&boss, &id, true)).await.0,
        StatusCode::NO_CONTENT
    );
    let rows: serde_json::Value = serde_json::from_str(&send(&app, get(&boss)).await.2).unwrap();
    assert_eq!(rows[0]["llm_ok"], true);
    assert_eq!(files(dir.path()), 1);
    assert_eq!(
        send(&app, delete(&boss, &id)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(files(dir.path()), 0);
    assert_eq!(send(&app, get(&boss)).await.2, "[]");
    assert_eq!(
        send(&app, delete(&boss, &id)).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn documents_need_a_session_and_a_safe_id() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (s, _, _) = send(
        &app,
        Request::get("/api/documents").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let boss = login(&app, "boss@example.com").await;
    assert_eq!(
        send(&app, delete(&boss, "..%2Fx")).await.0,
        StatusCode::NOT_FOUND
    );
}

fn usage(cookie: &str) -> Request<Body> {
    req(
        "GET",
        "/api/documents/usage",
        cookie,
        Body::empty(),
        "text/plain",
    )
}

#[tokio::test]
async fn documents_list_their_size_and_date_and_usage_counts_only_my_own() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let boss = login(&app, "boss@example.com").await;
    let amy = login(&app, "amy@example.com").await;
    let used = |body: &str| {
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        (
            v["used_bytes"].as_i64().unwrap(),
            v["cap_bytes"].as_i64().unwrap(),
        )
    };
    let cap = 500 * 1024 * 1024;
    assert_eq!(used(&send(&app, usage(&boss)).await.2), (0, cap));

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let (_, _, a) = send(&app, upload(&boss, "a.md", b"12345")).await;
    let (_, _, b) = send(&app, upload(&boss, "b.txt", b"1234567890")).await;
    send(&app, upload(&amy, "amy.md", b"123")).await;

    let rows: serde_json::Value = serde_json::from_str(&send(&app, get(&boss)).await.2).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    let size = |id: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap()["size_bytes"]
            .as_i64()
            .unwrap()
    };
    assert_eq!((size(&id_of(&a)), size(&id_of(&b))), (5, 10));
    for r in rows.as_array().unwrap() {
        let at = r["created_at"].as_i64().unwrap();
        assert!(
            at >= before && at <= before + 60_000,
            "created_at {at} is not now"
        );
    }

    // Each member's total is their own documents only.
    assert_eq!(used(&send(&app, usage(&boss)).await.2), (15, cap));
    assert_eq!(used(&send(&app, usage(&amy)).await.2), (3, cap));

    // The total follows a delete.
    send(&app, delete(&boss, &id_of(&b))).await;
    assert_eq!(used(&send(&app, usage(&boss)).await.2), (5, cap));
    assert_eq!(used(&send(&app, usage(&amy)).await.2), (3, cap));

    // A session is needed, and the usage path is not mistaken for a document id: PATCH and DELETE
    // on it are refused, and the listing is untouched.
    assert_eq!(
        send(&app, usage("recapz_session=nope")).await.0,
        StatusCode::UNAUTHORIZED
    );
    for method in ["PATCH", "DELETE"] {
        let (s, ..) = send(
            &app,
            req(
                method,
                "/api/documents/usage",
                &boss,
                Body::from(r#"{"llm_ok":true}"#),
                "application/json",
            ),
        )
        .await;
        assert_eq!(
            s,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} on the usage path"
        );
    }
    let rows: serde_json::Value = serde_json::from_str(&send(&app, get(&boss)).await.2).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
}
