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

fn req(method: &str, cookie: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri("/api/settings")
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

const DEFAULT: &str = r#"{"join_rule":"manual","notetaker_name":"Recapz notetaker","join_message":"Recapz notetaker is recording this call and transcribing it live. Type 'opt out' to make it leave and delete the recording."}"#;

#[tokio::test]
async fn settings_have_defaults_admin_updates_member_refused_and_bad_input_rejected() {
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
        ..Config::default()
    };
    let app = routes::router(cfg, Arc::new(Mutex::new(control)));
    let boss = send(&app, login("boss@example.com")).await.1.unwrap();
    let amy = send(&app, login("amy@example.com")).await.1.unwrap();

    let (s, _, b) = send(&app, req("GET", &amy, "")).await;
    assert_eq!((s, b.as_str()), (StatusCode::OK, DEFAULT));

    let new = r#"{"join_rule":"auto","notetaker_name":"Scribe","join_message":"Recording. Type opt out to leave."}"#;
    let (s, _, b) = send(&app, req("PUT", &amy, new)).await;
    assert_eq!(
        (s, b.as_str()),
        (StatusCode::FORBIDDEN, "only an admin can change settings")
    );
    let (_, _, b) = send(&app, req("GET", &boss, "")).await;
    assert_eq!(b, DEFAULT, "a refused change saves nothing");

    let (s, _, b) = send(&app, req("PUT", &boss, new)).await;
    assert_eq!((s, b.as_str()), (StatusCode::OK, new));
    let (s, _, b) = send(&app, req("GET", &amy, "")).await;
    assert_eq!((s, b.as_str()), (StatusCode::OK, new));

    let long_name = "n".repeat(61);
    let long_msg = format!("opt out {}", "m".repeat(493));
    let name_err = "notetaker name must be 1 to 60 characters";
    let cases = [
        (
            r#"{"join_rule":"everyone","notetaker_name":"S","join_message":"opt out"}"#.to_owned(),
            "join rule must be manual or auto",
        ),
        (
            r#"{"join_rule":"auto","notetaker_name":"","join_message":"opt out"}"#.to_owned(),
            name_err,
        ),
        (
            format!(
                r#"{{"join_rule":"auto","notetaker_name":"{long_name}","join_message":"opt out"}}"#
            ),
            name_err,
        ),
        (
            format!(r#"{{"join_rule":"auto","notetaker_name":"S","join_message":"{long_msg}"}}"#),
            "join message must be at most 500 characters",
        ),
        (
            r#"{"join_rule":"auto","notetaker_name":"S","join_message":"hello"}"#.to_owned(),
            "join message must contain the words \"opt out\"",
        ),
    ];
    assert_eq!(cases.len(), 5);
    for (body, msg) in cases {
        let (s, _, b) = send(&app, req("PUT", &boss, &body)).await;
        assert_eq!((s, b.as_str()), (StatusCode::BAD_REQUEST, msg), "{body}");
    }
    let (_, _, b) = send(&app, req("GET", &boss, "")).await;
    assert_eq!(b, new, "rejected input changes nothing");
}

#[tokio::test]
async fn default_share_is_admin_only_validated_and_needs_csrf() {
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
        ..Config::default()
    };
    let app = routes::router(cfg, Arc::new(Mutex::new(control)));
    let boss = send(&app, login("boss@example.com")).await.1.unwrap();
    let amy = send(&app, login("amy@example.com")).await.1.unwrap();
    let call = |method: &str, cookie: &str, body: &str, csrf: bool| {
        let mut b = Request::builder()
            .method(method)
            .uri("/api/settings/default-share")
            .header(header::CONTENT_TYPE, "application/json");
        b = if csrf {
            b.header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
                .header("x-csrf-token", "t")
        } else {
            b.header(header::COOKIE, cookie)
        };
        b.body(Body::from(body.to_string())).unwrap()
    };
    let attendees = r#"{"share":"attendees"}"#;
    let workspace = r#"{"share":"workspace"}"#;

    let (s, _, b) = send(&app, call("GET", &amy, "", true)).await;
    assert_eq!((s, b.as_str()), (StatusCode::OK, attendees));
    let (s, _, _) = send(&app, call("PUT", &amy, workspace, true)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = send(&app, call("PUT", &boss, workspace, false)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "no CSRF token");
    let (s, _, b) = send(&app, call("PUT", &boss, r#"{"share":"everyone"}"#, true)).await;
    assert_eq!(
        (s, b.as_str()),
        (
            StatusCode::BAD_REQUEST,
            "share must be owner, attendees or workspace"
        )
    );
    let (_, _, b) = send(&app, call("GET", &boss, "", true)).await;
    assert_eq!(b, attendees, "refused changes save nothing");
    let (s, _, b) = send(&app, call("PUT", &boss, workspace, true)).await;
    assert_eq!((s, b.as_str()), (StatusCode::OK, workspace));
    let (_, _, b) = send(&app, call("GET", &amy, "", true)).await;
    assert_eq!(b, workspace);
}
