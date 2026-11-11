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

fn get(path: &str, cookie: &str) -> Request<Body> {
    Request::get(path)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

fn upload(cookie: &str, title: &str) -> Request<Body> {
    let b = "XBOUNDARY";
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\n{title}\r\n--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFdata\r\n--{b}--\r\n"
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
async fn upload_creates_a_meeting_and_a_job_visible_only_to_its_owner() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();

    let (s, ..) = send(&app, upload(&boss, "Weekly sync")).await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, _, body) = send(&app, get("/api/meetings", &boss)).await;
    let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["title"], "Weekly sync");
    assert_eq!(rows[0]["status"], "uploaded");

    // A transcribe job was queued and the audio file was stored under the tenant folder.
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let jobs: i64 = t
        .query_row(
            "SELECT count(*) FROM jobs WHERE kind='transcribe'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(jobs, 1);
    let path: String = t
        .query_row("SELECT audio_path FROM meetings", [], |r| r.get(0))
        .unwrap();
    assert!(db::tenant_dir(dir.path(), &tid).join(path).exists());

    // Another member of the same workspace cannot see it.
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let (s, _, body) = send(&app, get("/api/meetings", &amy.unwrap())).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, "[]"));

    // Signed-out uploads are rejected.
    let (s, ..) = send(&app, upload("recapz_session=nope", "x")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn meeting_detail_and_transcript_follow_the_access_rule() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    send(&app, upload(&boss, "Weekly sync")).await;

    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let id: String = t
        .query_row("SELECT id FROM meetings", [], |r| r.get(0))
        .unwrap();
    t.execute_batch(&format!(
        "INSERT INTO segments (meeting_id, start_ms, end_ms, text) VALUES
           ('{id}', 2000, 3000, 'second'), ('{id}', 0, 2000, 'first')"
    ))
    .unwrap();
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();

    // Owner: detail plus segments in time order.
    let (s, _, body) = send(&app, get(&format!("/api/meetings/{id}"), &boss)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains(r#""title":"Weekly sync""#), "{body}");
    let (s, _, body) = send(&app, get(&format!("/api/meetings/{id}/transcript"), &boss)).await;
    assert_eq!(s, StatusCode::OK);
    let segs: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(segs.as_array().unwrap().len(), 2);
    assert_eq!(
        (segs[0]["text"].as_str(), segs[1]["text"].as_str()),
        (Some("first"), Some("second"))
    );

    // Another member gets 404 for both, as if it did not exist.
    for path in [
        format!("/api/meetings/{id}"),
        format!("/api/meetings/{id}/transcript"),
    ] {
        let (s, ..) = send(&app, get(&path, &amy)).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{path}");
    }
    // Unknown id and signed-out requests.
    let (s, ..) = send(&app, get("/api/meetings/nope", &boss)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, ..) = send(
        &app,
        get(&format!("/api/meetings/{id}"), "recapz_session=nope"),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Sharing with the workspace opens it up.
    t.execute("UPDATE meetings SET share='workspace'", [])
        .unwrap();
    let (s, ..) = send(&app, get(&format!("/api/meetings/{id}/transcript"), &amy)).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn audio_supports_ranges_and_follows_the_access_rule() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    send(&app, upload(&boss, "Weekly sync")).await; // stored bytes: "RIFFdata"

    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let id: String = t
        .query_row("SELECT id FROM meetings", [], |r| r.get(0))
        .unwrap();
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    let path = format!("/api/meetings/{id}/audio");
    let ranged = |range: &str| {
        Request::get(&path)
            .header(header::COOKIE, &boss)
            .header(header::RANGE, range)
            .body(Body::empty())
            .unwrap()
    };

    let res = app.clone().oneshot(get(&path, &boss)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::ACCEPT_RANGES], "bytes");
    assert_eq!(res.headers()[header::CONTENT_TYPE], "audio/wav");
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"RIFFdata");

    let res = app.clone().oneshot(ranged("bytes=4-6")).await.unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(res.headers()[header::CONTENT_RANGE], "bytes 4-6/8");
    assert_eq!(
        &res.into_body().collect().await.unwrap().to_bytes()[..],
        b"dat"
    );

    let res = app.clone().oneshot(ranged("bytes=-2")).await.unwrap();
    assert_eq!(res.headers()[header::CONTENT_RANGE], "bytes 6-7/8");
    let res = app.clone().oneshot(ranged("bytes=99-")).await.unwrap();
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(res.headers()[header::CONTENT_RANGE], "bytes */8");

    let (s, ..) = send(&app, get(&path, &amy)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, ..) = send(&app, get(&path, "recapz_session=nope")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

fn json_req(method: &str, path: &str, cookie: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn upcoming_events_are_private_and_the_join_switch_needs_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    // The handlers create the member rows on first use.
    send(&app, get("/api/meetings/upcoming", &boss)).await;
    send(&app, get("/api/meetings/upcoming", &amy)).await;

    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let boss_id: String = t
        .query_row(
            "SELECT id FROM members WHERE email='boss@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let far = db::now_ms() + 3_600_000;
    let ev = |ext: &str, url: Option<&str>| recapz_core::calendar::NewEvent {
        external_id: ext.into(),
        title: format!("Standup {ext}"),
        starts_at: far,
        ends_at: far + 1000,
        join_url: url.map(Into::into),
        platform: url.map(|_| "meet".into()),
    };
    recapz_core::calendar::sync(
        &t,
        &boss_id,
        &[
            ev("a", Some("https://meet.google.com/abc-defg-hij")),
            ev("b", None),
        ],
        0,
        far * 2,
        false,
    )
    .unwrap();

    let (s, _, body) = send(&app, get("/api/meetings/upcoming", &boss)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        !body.contains("meet.google.com"),
        "the link must not reach the browser: {body}"
    );
    let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(
        (
            rows[0]["has_link"].as_bool(),
            rows[0]["auto_join"].as_bool()
        ),
        (Some(true), Some(false))
    );
    assert_eq!(rows[1]["has_link"], false);
    let (a, b) = (
        rows[0]["id"].as_str().unwrap().to_string(),
        rows[1]["id"].as_str().unwrap().to_string(),
    );

    let (s, _, body) = send(&app, get("/api/meetings/upcoming", &amy)).await;
    assert_eq!(
        (s, body.as_str()),
        (StatusCode::OK, "[]"),
        "another member sees none of it"
    );

    let on = r#"{"auto_join":true}"#;
    let (s, ..) = send(
        &app,
        json_req("PATCH", &format!("/api/calendar-events/{a}"), &amy, on),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "not amy's event");
    let (s, ..) = send(
        &app,
        json_req("PATCH", &format!("/api/calendar-events/{b}"), &boss, on),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "no link, nothing to join");
    let (s, ..) = send(
        &app,
        json_req("PATCH", &format!("/api/calendar-events/{a}"), &boss, on),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, _, body) = send(&app, get("/api/meetings/upcoming", &boss)).await;
    let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows[0]["auto_join"], true);
    let (s, ..) = send(
        &app,
        json_req(
            "PATCH",
            &format!("/api/calendar-events/{a}"),
            "recapz_session=nope",
            on,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn joining_a_pasted_link_queues_a_scheduled_meeting_for_the_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();

    let (s, _, body) = send(
        &app,
        json_req("POST", "/api/meetings/join", &boss, r#"{"url":"https://us02web.zoom.us/j/84012345678?pwd=abc","title":"  Design review  "}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, _, body) = send(&app, get(&format!("/api/meetings/{id}"), &boss)).await;
    let m: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (
            m["title"].as_str(),
            m["platform"].as_str(),
            m["status"].as_str()
        ),
        (Some("Design review"), Some("zoom"), Some("scheduled"))
    );

    let (s, _, body) = send(
        &app,
        json_req(
            "POST",
            "/api/meetings/join",
            &boss,
            r#"{"url":"https://meet.google.com/abc-defg-hij"}"#,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let id2 = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, _, body) = send(&app, get(&format!("/api/meetings/{id2}"), &boss)).await;
    assert!(
        body.contains(r#""title":"Meet call""#) && body.contains(r#""platform":"meet""#),
        "{body}"
    );

    // Sending the same link again returns the queued meeting instead of a second one.
    let (s, _, again) = send(
        &app,
        json_req(
            "POST",
            "/api/meetings/join",
            &boss,
            r#"{"url":"https://meet.google.com/abc-defg-hij?authuser=2"}"#,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&again).unwrap()["id"],
        id2.as_str()
    );
    let long = format!(
        r#"{{"url":"https://us02web.zoom.us/j/84012345679","title":"{}"}}"#,
        "x".repeat(300)
    );
    let (s, _, body) = send(&app, json_req("POST", "/api/meetings/join", &boss, &long)).await;
    assert_eq!(s, StatusCode::CREATED);
    let id3 = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, _, body) = send(&app, get(&format!("/api/meetings/{id3}"), &boss)).await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["title"]
            .as_str()
            .unwrap()
            .len(),
        200
    );

    for bad in [
        r#"{"url":"https://meet.google.com.evil.com/abc-defg-hij"}"#,
        r#"{"url":"http://meet.google.com/abc-defg-hij"}"#,
        r#"{"url":"https://example.com/"}"#,
    ] {
        let (s, ..) = send(&app, json_req("POST", "/api/meetings/join", &boss, bad)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (_, _, body) = send(&app, get("/api/meetings", &boss)).await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        3,
        "rejected links and repeats create nothing"
    );
    let (s, ..) = send(
        &app,
        json_req(
            "POST",
            "/api/meetings/join",
            "recapz_session=nope",
            r#"{"url":"https://meet.google.com/abc-defg-hij"}"#,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}
