use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use recapz_core::db;
use recapz_server::{config::Config, routes};
use serde_json::json;
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
    assert!(!body.contains("speech_provider"), "{body}");

    let logout = Request::post("/auth/logout")
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
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
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .body(Body::empty())
        .unwrap()
}

fn upload(cookie: &str, title: &str) -> Request<Body> {
    let b = "XBOUNDARY";
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\n{title}\r\n--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFdata\r\n--{b}--\r\n"
    );
    Request::post("/api/uploads")
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
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

#[tokio::test]
async fn report_is_null_until_written_and_follows_the_access_rule() {
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
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    let path = format!("/api/meetings/{id}/report");

    let (s, _, body) = send(&app, get(&path, &boss)).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, "null"));

    t.execute_batch(&format!(
        "INSERT INTO reports (meeting_id, summary, decisions_json, questions_json, chapters_json, model, created_at)
           VALUES ('{id}', 'Sum', '[\"d1\"]', '[]', '[{{\"start_ms\":5,\"title\":\"Intro\"}}]', 'm', 9);
         INSERT INTO action_items (id, meeting_id, text, due_date, source_ms) VALUES
           ('a2', '{id}', 'later', NULL, 90000), ('a1', '{id}', 'first', '2026-12-01', 1000);"
    ))
    .unwrap();
    let (s, _, body) = send(&app, get(&path, &boss)).await;
    assert_eq!(s, StatusCode::OK);
    let r: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(r["summary"], "Sum");
    assert_eq!(r["decisions"], serde_json::json!(["d1"]));
    assert_eq!(
        r["chapters"],
        serde_json::json!([{"start_ms": 5, "title": "Intro"}])
    );
    let items = r["action_items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(
        (
            items[0]["text"].as_str(),
            items[0]["due_date"].as_str(),
            items[0]["done"].as_bool()
        ),
        (Some("first"), Some("2026-12-01"), Some(false))
    );
    assert_eq!(items[1]["text"], "later");

    let (s, ..) = send(&app, get(&path, &amy)).await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "private meeting must not leak its report"
    );
    t.execute("UPDATE meetings SET share='workspace'", [])
        .unwrap();
    let (s, ..) = send(&app, get(&path, &amy)).await;
    assert_eq!(s, StatusCode::OK);
    let (s, ..) = send(&app, get(&path, "recapz_session=nope")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

fn json_req(method: &str, path: &str, cookie: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
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
        attendees: if ext == "a" {
            vec![recapz_core::calendar::Attendee {
                name: Some("Ann Lee".into()),
                email: "ann@example.com".into(),
                status: Some("accepted".into()),
            }]
        } else {
            vec![]
        },
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
    let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(
        (rows[0]["join_url"].as_str(), rows[0]["auto_join"].as_bool()),
        (Some("https://meet.google.com/abc-defg-hij"), Some(false))
    );
    assert!(rows[1]["join_url"].is_null());
    assert_eq!(
        rows[0]["attendees"],
        serde_json::json!([{"name": "Ann Lee", "email": "ann@example.com", "status": "accepted"}])
    );
    assert_eq!(rows[1]["attendees"], serde_json::json!([]));
    let (a, b) = (
        rows[0]["id"].as_str().unwrap().to_string(),
        rows[1]["id"].as_str().unwrap().to_string(),
    );

    let (s, _, body) = send(&app, get("/api/meetings/upcoming", &amy)).await;
    assert_eq!(
        (s, body.as_str()),
        (StatusCode::OK, "[]"),
        "another member sees none of it, attendees included"
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

fn del(path: &str, cookie: &str) -> Request<Body> {
    json_req("DELETE", path, cookie, "")
}

fn count(t: &rusqlite::Connection, sql: &str) -> i64 {
    t.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// Adds one row in every table a meeting owns, plus an audio file; returns the meeting id.
fn seed_full_meeting(
    t: &rusqlite::Connection,
    dir: &std::path::Path,
    tid: &str,
    owner: &str,
    status: &str,
) -> String {
    let id = format!("m-{owner}-{status}");
    let audio = db::tenant_dir(dir, tid).join("audio").join(&id);
    std::fs::create_dir_all(&audio).unwrap();
    std::fs::write(audio.join("a.wav"), b"RIFF").unwrap();
    t.execute_batch(&format!(
        "INSERT INTO meetings (id, title, platform, status, owner_id, share, audio_path) VALUES ('{id}','t','upload','{status}','{owner}','attendees','audio/{id}/a.wav');
         INSERT INTO speakers (id, meeting_id, label) VALUES ('s-{id}','{id}','Speaker 1');
         INSERT INTO segments (meeting_id, speaker_id, start_ms, end_ms, text) VALUES ('{id}','s-{id}',0,1,'zebrafish');
         INSERT INTO reports (meeting_id, summary) VALUES ('{id}','zebrafish');
         INSERT INTO action_items (id, meeting_id, text) VALUES ('a-{id}','{id}','zebrafish');
         INSERT INTO chunks (meeting_id, start_ms, end_ms, text, embedding) VALUES ('{id}',0,1,'zebrafish',x'00');
         INSERT INTO jobs (kind, meeting_id, run_after) VALUES ('report','{id}',0);"
    ))
    .unwrap();
    id
}

#[tokio::test]
async fn transcript_returns_each_segments_speaker_name() {
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
        "INSERT INTO speakers (id, meeting_id, label, display_name) VALUES
           ('s1','{id}','Speaker 1','Lan'), ('s2','{id}','Speaker 2','Minh');
         INSERT INTO segments (meeting_id, speaker_id, start_ms, end_ms, text) VALUES
           ('{id}','s1',0,1000,'a'), ('{id}','s2',1000,2000,'b'), ('{id}',NULL,2000,3000,'c')"
    ))
    .unwrap();
    let (s, _, body) = send(&app, get(&format!("/api/meetings/{id}/transcript"), &boss)).await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 3);
    let got: Vec<_> = (0..3)
        .map(|i| (v[i]["speaker_id"].clone(), v[i]["speaker_name"].clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            (json!("s1"), json!("Lan")),
            (json!("s2"), json!("Minh")),
            (json!(null), json!(null))
        ]
    );
}

#[tokio::test]
async fn regenerate_queues_one_report_job_for_a_ready_meeting_owner_or_admin_only() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    for e in ["amy@example.com", "bob@example.com"] {
        control
            .execute(
                "INSERT INTO invites (email, tenant_id, role) VALUES (?1, ?2, 'member')",
                rusqlite::params![e, tid],
            )
            .unwrap();
    }
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    let (_, bob, _) = send(&app, login("bob@example.com")).await;
    let bob = bob.unwrap();
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    for c in [&boss, &amy, &bob] {
        send(&app, get("/api/meetings", c)).await;
    }
    let amy_id: String = t
        .query_row(
            "SELECT id FROM members WHERE email='amy@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let ready = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "ready");
    let busy = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "transcribing");
    let url = |id: &str| format!("/api/meetings/{id}/report/regenerate");
    let jobs = |id: &str| {
        count(
            &t,
            &format!("SELECT count(*) FROM jobs WHERE meeting_id='{id}'"),
        )
    };
    let status = |id: &str| -> String {
        t.query_row("SELECT status FROM meetings WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    };

    let (s, ..) = send(&app, json_req("POST", &url(&ready), &bob, "")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, body) = send(&app, json_req("POST", &url(&busy), &amy, "")).await;
    assert_eq!(
        (s, body.as_str()),
        (StatusCode::CONFLICT, "meeting is not ready")
    );
    assert_eq!(
        (jobs(&ready), jobs(&busy)),
        (1, 1),
        "refusals queue nothing"
    );

    // Owner queues one job; the old report stays; a second click is refused while it runs.
    let (s, ..) = send(&app, json_req("POST", &url(&ready), &amy, "")).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert_eq!((jobs(&ready), status(&ready).as_str()), (2, "reporting"));
    assert_eq!(
        count(
            &t,
            &format!("SELECT count(*) FROM reports WHERE meeting_id='{ready}'")
        ),
        1
    );
    let (s, ..) = send(&app, json_req("POST", &url(&ready), &amy, "")).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(jobs(&ready), 2);

    // A member who can see the meeting but does not own it is denied; an admin may regenerate.
    t.execute(
        "UPDATE meetings SET status='ready', share='workspace' WHERE id=?1",
        [&ready],
    )
    .unwrap();
    let (s, ..) = send(&app, json_req("POST", &url(&ready), &bob, "")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, ..) = send(&app, json_req("POST", &url(&ready), &boss, "")).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert_eq!(jobs(&ready), 3);
}

#[tokio::test]
async fn retry_requeues_the_failed_job_for_owner_or_admin_only() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    for e in ["amy@example.com", "bob@example.com"] {
        control
            .execute(
                "INSERT INTO invites (email, tenant_id, role) VALUES (?1, ?2, 'member')",
                rusqlite::params![e, tid],
            )
            .unwrap();
    }
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    let (_, bob, _) = send(&app, login("bob@example.com")).await;
    let bob = bob.unwrap();
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    for c in [&boss, &amy, &bob] {
        send(&app, get("/api/meetings", c)).await;
    }
    let amy_id: String = t
        .query_row(
            "SELECT id FROM members WHERE email='amy@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let failed = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "failed");
    let ready = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "ready");
    t.execute(
        "UPDATE jobs SET state='failed', attempts=3 WHERE meeting_id=?1",
        [&failed],
    )
    .unwrap();
    t.execute("UPDATE meetings SET error='boom' WHERE id=?1", [&failed])
        .unwrap();
    let url = |id: &str| format!("/api/meetings/{id}/retry");
    let job = || -> (String, i64) {
        t.query_row(
            "SELECT state, attempts FROM jobs WHERE meeting_id=?1",
            [&failed],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };

    let (s, ..) = send(&app, json_req("POST", &url(&failed), &bob, "")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, body) = send(&app, json_req("POST", &url(&ready), &amy, "")).await;
    assert_eq!(
        (s, body.as_str()),
        (StatusCode::CONFLICT, "meeting has not failed")
    );
    assert_eq!(job(), ("failed".to_string(), 3), "refusals queue nothing");

    t.execute(
        "UPDATE meetings SET share='workspace' WHERE id=?1",
        [&failed],
    )
    .unwrap();
    let (s, ..) = send(&app, json_req("POST", &url(&failed), &bob, "")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, ..) = send(&app, json_req("POST", &url(&failed), &amy, "")).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert_eq!(job(), ("queued".to_string(), 0));
    let m: (String, Option<String>) = t
        .query_row(
            "SELECT status, error FROM meetings WHERE id=?1",
            [&failed],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(m, ("reporting".to_string(), None));
    let (s, ..) = send(&app, json_req("POST", &url(&failed), &boss, "")).await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "a second click finds nothing failed"
    );
}

#[tokio::test]
async fn delete_meeting_is_owner_or_admin_only_and_removes_everything() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    for e in ["amy@example.com", "bob@example.com"] {
        control
            .execute(
                "INSERT INTO invites (email, tenant_id, role) VALUES (?1, ?2, 'member')",
                rusqlite::params![e, tid],
            )
            .unwrap();
    }
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    let (_, bob, _) = send(&app, login("bob@example.com")).await;
    let bob = bob.unwrap();
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let uid = |email: &str| -> String {
        t.query_row("SELECT id FROM members WHERE email = ?1", [email], |r| {
            r.get(0)
        })
        .unwrap()
    };
    // members rows appear on first request; touch each once
    for c in [&boss, &amy, &bob] {
        send(&app, get("/api/meetings", c)).await;
    }
    let (amy_id, boss_id) = (uid("amy@example.com"), uid("boss@example.com"));

    let mine = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "ready");
    let keep = seed_full_meeting(&t, dir.path(), &tid, &boss_id, "ready");
    let busy = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "transcribing");
    let shared = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "scheduled");
    t.execute(
        "UPDATE meetings SET share='workspace' WHERE id=?1",
        [&shared],
    )
    .unwrap();
    let private = |id: &str| format!("/api/meetings/{id}");

    // Signed out, unseen private meeting, still processing, non-owner member: nothing is deleted.
    let (s, ..) = send(&app, del(&private(&mine), "recapz_session=nope")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, ..) = send(&app, del(&private(&mine), &bob)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, body) = send(&app, del(&private(&busy), &amy)).await;
    assert_eq!(
        (s, body.as_str()),
        (StatusCode::CONFLICT, "meeting is still processing")
    );
    let (s, ..) = send(&app, del(&private(&shared), &bob)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(count(&t, "SELECT count(*) FROM meetings"), 4);

    // The owner deletes: rows, search rows and audio are gone, the other meetings are untouched.
    let (s, ..) = send(&app, del(&private(&mine), &amy)).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    for table in [
        "segments",
        "speakers",
        "reports",
        "action_items",
        "chunks",
        "jobs",
    ] {
        assert_eq!(
            count(
                &t,
                &format!("SELECT count(*) FROM {table} WHERE meeting_id='{mine}'")
            ),
            0,
            "{table}"
        );
        assert_eq!(
            count(
                &t,
                &format!("SELECT count(*) FROM {table} WHERE meeting_id='{keep}'")
            ),
            1,
            "{table} of the other meeting"
        );
    }
    assert_eq!(count(&t, "SELECT count(*) FROM meetings"), 3);
    // Four seeded meetings each had one search row; the deleted one's is gone.
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'zebrafish'"
        ),
        3
    );
    assert!(!db::tenant_dir(dir.path(), &tid)
        .join("audio")
        .join(&mine)
        .exists());
    assert!(db::tenant_dir(dir.path(), &tid)
        .join("audio")
        .join(&keep)
        .join("a.wav")
        .exists());
    let (s, ..) = send(&app, get(&private(&mine), &amy)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // An admin may delete a workspace-shared meeting they do not own.
    let (s, ..) = send(&app, del(&private(&shared), &boss)).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(count(&t, "SELECT count(*) FROM meetings"), 2);
}

#[tokio::test]
async fn rename_meeting_trims_validates_and_is_owner_or_admin_only() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    for e in ["amy@example.com", "bob@example.com"] {
        control
            .execute(
                "INSERT INTO invites (email, tenant_id, role) VALUES (?1, ?2, 'member')",
                rusqlite::params![e, tid],
            )
            .unwrap();
    }
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    let (_, bob, _) = send(&app, login("bob@example.com")).await;
    let bob = bob.unwrap();
    for c in [&boss, &amy, &bob] {
        send(&app, get("/api/meetings", c)).await;
    }
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let amy_id: String = t
        .query_row(
            "SELECT id FROM members WHERE email = 'amy@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mine = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "transcribing");
    let other = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "ready");
    t.execute(
        "UPDATE meetings SET share='workspace' WHERE id=?1",
        [&other],
    )
    .unwrap();
    let title = |id: &str| -> String {
        t.query_row("SELECT title FROM meetings WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    };
    let path = format!("/api/meetings/{mine}");
    let patch = |p: &str, c: &str, b: &str| json_req("PATCH", p, c, b);

    // Signed out, unseen private meeting, unknown id, bad titles: nothing changes.
    let (s, ..) = send(
        &app,
        patch(&path, "recapz_session=nope", r#"{"title":"x"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, ..) = send(&app, patch(&path, &bob, r#"{"title":"x"}"#)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, ..) = send(&app, patch("/api/meetings/nope", &amy, r#"{"title":"x"}"#)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    for body in [r#"{"title":"   "}"#, r#"{"title":""}"#] {
        let (s, ..) = send(&app, patch(&path, &amy, body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    }
    let long = format!(r#"{{"title":"{}"}}"#, "a".repeat(201));
    let (s, ..) = send(&app, patch(&path, &amy, &long)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(title(&mine), "t");
    // A member who can see a shared meeting but does not own it may not rename it.
    let (s, ..) = send(
        &app,
        patch(&format!("/api/meetings/{other}"), &bob, r#"{"title":"x"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(title(&other), "t");

    // The owner renames (trimmed, 200 characters is allowed, even mid-processing); the response and the detail agree.
    let (s, _, body) = send(&app, patch(&path, &amy, r#"{"title":"  Q3 planning  "}"#)).await;
    assert_eq!(
        (s, body.as_str()),
        (StatusCode::OK, r#"{"title":"Q3 planning"}"#)
    );
    assert_eq!(title(&mine), "Q3 planning");
    let (_, _, detail) = send(&app, get(&path, &amy)).await;
    assert!(detail.contains(r#""title":"Q3 planning""#), "{detail}");
    let max = format!(r#"{{"title":"{}"}}"#, "é".repeat(200));
    let (s, ..) = send(&app, patch(&path, &amy, &max)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(title(&mine).chars().count(), 200);
    // An admin may rename a workspace-shared meeting they do not own; the other meeting is untouched.
    let (s, ..) = send(
        &app,
        patch(
            &format!("/api/meetings/{other}"),
            &boss,
            r#"{"title":"Shared"}"#,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (title(&other), title(&mine).chars().count()),
        ("Shared".into(), 200)
    );
}

#[tokio::test]
async fn meeting_list_returns_duration_action_count_and_capped_speakers() {
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
        "UPDATE meetings SET started_at = 1000, ended_at = 61000 WHERE id = '{id}';
         INSERT INTO action_items (id, meeting_id, text) VALUES ('a1','{id}','x'), ('a2','{id}','y');
         INSERT INTO speakers (id, meeting_id, label, display_name) VALUES
           ('p1','{id}','Speaker 1','Lan'), ('p2','{id}','Speaker 2',NULL), ('p3','{id}','S3',NULL),
           ('p4','{id}','S4',NULL), ('p5','{id}','S5',NULL), ('p6','{id}','S6',NULL);
         INSERT INTO members (id, email, role) VALUES ('someone-else','se@example.com','member');
         INSERT INTO meetings (id, title, platform, status, owner_id, share)
           VALUES ('other','o','upload','done','someone-else','attendees');
         INSERT INTO speakers (id, meeting_id, label) VALUES ('px','other','Hidden');"
    ))
    .unwrap();
    let (_, _, body) = send(&app, get("/api/meetings", &boss)).await;
    let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["duration_ms"], 60000);
    assert_eq!(rows[0]["action_item_count"], 2);
    assert_eq!(
        rows[0]["speakers"],
        serde_json::json!([
            {"id":"p1","name":"Lan"},{"id":"p2","name":"Speaker 2"},{"id":"p3","name":"S3"},
            {"id":"p4","name":"S4"},{"id":"p5","name":"S5"}])
    );

    // The detail returns the same speakers, so both screens colour from one order.
    let (_, _, body) = send(&app, get(&format!("/api/meetings/{id}"), &boss)).await;
    let detail: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(detail["speakers"], rows[0]["speakers"]);
    assert_eq!(detail["speakers"].as_array().unwrap().len(), 5);
    assert_eq!(
        detail["speakers"][0],
        serde_json::json!({"id":"p1","name":"Lan"})
    );

    // Unknown duration and no report: null, 0 and an empty list.
    t.execute_batch(&format!(
        "UPDATE meetings SET ended_at = NULL WHERE id = '{id}';
         DELETE FROM action_items; DELETE FROM speakers;"
    ))
    .unwrap();
    let (_, _, body) = send(&app, get("/api/meetings", &boss)).await;
    let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows[0]["duration_ms"], serde_json::Value::Null);
    assert_eq!(rows[0]["action_item_count"], 0);
    assert_eq!(rows[0]["speakers"], serde_json::json!([]));
}

#[tokio::test]
async fn audio_retention_defaults_to_90_days_and_only_an_admin_changes_it() {
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

    let path = "/api/settings/audio-retention";
    let (s, _, body) = send(&app, get(path, &amy)).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, r#"{"days":90}"#));
    let (s, ..) = send(&app, json_req("PUT", path, &amy, r#"{"days":1}"#)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, body) = send(&app, json_req("PUT", path, &boss, r#"{"days":30}"#)).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, r#"{"days":30}"#));
    let (s, _, body) = send(&app, get(path, &amy)).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, r#"{"days":30}"#));
    let (s, ..) = send(&app, json_req("PUT", path, &boss, r#"{"days":99999}"#)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, _, body) = send(&app, get(path, &boss)).await;
    assert_eq!(body, r#"{"days":30}"#, "a rejected value changes nothing");
    let (_, _, me) = send(&app, get("/api/me", &boss)).await;
    assert!(me.contains(r#""recording_storage":"local_disk""#), "{me}");
}

#[tokio::test]
async fn usage_is_admin_only_and_reports_cost_cap_and_pause() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config {
        data_dir: dir.path().into(),
        dev_login: true,
        monthly_cap_usd: 0.0001, // 100 micro-USD
        ..Config::default()
    };
    cfg.speech.price_per_minute_micro_usd = 7_800;
    let control = db::open_control(dir.path()).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    control
        .execute(
            "INSERT INTO invites (email, tenant_id, role) VALUES ('amy@example.com', ?1, 'member')",
            [&tid],
        )
        .unwrap();
    let app = routes::router(cfg, Arc::new(Mutex::new(control)));
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let (boss, amy) = (boss.unwrap(), amy.unwrap());
    let (s, ..) = send(&app, get("/api/usage", &amy)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let t = db::open_tenant(dir.path(), &tid).unwrap();
    t.execute("INSERT INTO meetings (id, title, platform, status, share) VALUES ('m1','Standup','upload','ready','workspace')", []).unwrap();
    let now = db::now_ms();
    t.execute(
        "INSERT INTO speech_usage (meeting_id, kind, audio_seconds, cost_micro_usd, at) VALUES ('m1','batch',60,70,?1)",
        [now],
    )
    .unwrap();
    let (s, _, body) = send(&app, get("/api/usage", &boss)).await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["total_micro_usd"], 70);
    assert_eq!(
        (v["cap_micro_usd"].as_u64(), v["paused"].as_bool()),
        (Some(100), Some(false))
    );
    assert_eq!(v["meetings"][0]["title"], "Standup");
    assert_eq!(v["meetings"][0]["speech_micro_usd"], 70);
    t.execute(
        "INSERT INTO llm_usage (meeting_id, task, provider, model, cost_micro_usd, at) VALUES ('m1','summary','p','m',30,?1)",
        [now],
    )
    .unwrap();
    let (_, _, body) = send(&app, get("/api/usage", &boss)).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (v["total_micro_usd"].as_i64(), v["paused"].as_bool()),
        (Some(100), Some(true)),
        "{body}"
    );
}

#[tokio::test]
async fn a_meeting_language_defaults_to_vietnamese_and_only_vi_en_multi_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let with_language = |lang: Option<&str>| {
        let b = "XBOUNDARY";
        let field = lang
            .map(|l| {
                format!("--{b}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n{l}\r\n")
            })
            .unwrap_or_default();
        Request::post("/api/uploads")
            .header(header::COOKIE, format!("{boss}; recapz_csrf=t"))
            .header("x-csrf-token", "t")
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={b}"))
            .body(Body::from(format!(
                "{field}--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFdata\r\n--{b}--\r\n"
            )))
            .unwrap()
    };
    for (sent, stored) in [
        (None, "vi"),
        (Some("en"), "en"),
        (Some("multi"), "multi"),
        (Some(" vi "), "vi"),
    ] {
        let (s, _, body) = send(&app, with_language(sent)).await;
        assert_eq!(s, StatusCode::CREATED, "{sent:?}");
        let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let control = db::open_control(dir.path()).unwrap();
        let tid = db::tenant_ids(&control).unwrap().remove(0);
        let t = db::open_tenant(dir.path(), &tid).unwrap();
        let lang: String = t
            .query_row("SELECT language FROM meetings WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(lang, stored, "{sent:?}");
    }
    for bad in ["fr", "vi+en", "auto"] {
        let (s, ..) = send(&app, with_language(Some(bad))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
    let n: i64 = db::open_tenant(
        dir.path(),
        &db::tenant_ids(&db::open_control(dir.path()).unwrap())
            .unwrap()
            .remove(0),
    )
    .unwrap()
    .query_row("SELECT count(*) FROM meetings", [], |r| r.get(0))
    .unwrap();
    assert_eq!(n, 4, "a rejected language creates no meeting");

    // the notetaker join flow takes the same field
    let join = |body: &str| json_req("POST", "/api/meetings/join", &boss, body);
    let (s, _, b1) = send(
        &app,
        join(r#"{"url":"https://meet.google.com/abc-defg-hij","language":"en"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b1}");
    let (s, ..) = send(
        &app,
        join(r#"{"url":"https://meet.google.com/zzz-yyyy-xxx","language":"fr"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, b3) = send(
        &app,
        join(r#"{"url":"https://meet.google.com/qqq-wwww-eee"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b3}");
    let t = db::open_tenant(
        dir.path(),
        &db::tenant_ids(&db::open_control(dir.path()).unwrap())
            .unwrap()
            .remove(0),
    )
    .unwrap();
    let langs: Vec<String> = t
        .prepare("SELECT language FROM meetings WHERE platform != 'upload' ORDER BY created_at, id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(langs.len(), 2);
    assert!(
        langs.contains(&"en".to_string()) && langs.contains(&"vi".to_string()),
        "{langs:?}"
    );
}

#[tokio::test]
async fn the_language_changes_only_before_the_notetaker_joins_and_only_by_owner_or_admin() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    for e in ["amy@example.com", "bob@example.com"] {
        control
            .execute(
                "INSERT INTO invites (email, tenant_id, role) VALUES (?1, ?2, 'member')",
                rusqlite::params![e, tid],
            )
            .unwrap();
    }
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    let (_, bob, _) = send(&app, login("bob@example.com")).await;
    let bob = bob.unwrap();
    for c in [&boss, &amy, &bob] {
        send(&app, get("/api/meetings", c)).await;
    }
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let amy_id: String = t
        .query_row(
            "SELECT id FROM members WHERE email = 'amy@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    t.execute_batch(&format!(
        "INSERT INTO meetings (id, title, platform, status, owner_id, share, language) VALUES
           ('soon','t','meet','scheduled','{amy_id}','workspace','vi'),
           ('live','t','meet','recording','{amy_id}','attendees','vi');"
    ))
    .unwrap();
    let lang = |id: &str| -> String {
        t.query_row("SELECT language FROM meetings WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    };
    let patch = |id: &str, c: &str, b: &str| {
        json_req("PATCH", &format!("/api/meetings/{id}/language"), c, b)
    };

    // Bad input, a stranger, and a non-owner who can see the meeting change nothing.
    for body in [r#"{"language":"fr"}"#, r#"{"language":""}"#] {
        let (s, ..) = send(&app, patch("soon", &amy, body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    }
    let (s, ..) = send(&app, patch("live", &bob, r#"{"language":"en"}"#)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, ..) = send(&app, patch("soon", &bob, r#"{"language":"en"}"#)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(lang("soon"), "vi");

    // Once recording, the language is locked.
    let (s, ..) = send(&app, patch("live", &amy, r#"{"language":"multi"}"#)).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(lang("live"), "vi");

    // The owner, then an admin, change a scheduled meeting; the detail agrees.
    let (s, _, body) = send(&app, patch("soon", &amy, r#"{"language":" multi "}"#)).await;
    assert_eq!(
        (s, body.as_str(), lang("soon").as_str()),
        (StatusCode::OK, r#"{"language":"multi"}"#, "multi")
    );
    let (s, ..) = send(&app, patch("soon", &boss, r#"{"language":"en"}"#)).await;
    assert_eq!((s, lang("soon").as_str()), (StatusCode::OK, "en"));
    let (_, _, detail) = send(&app, get("/api/meetings/soon", &amy)).await;
    assert!(detail.contains(r#""language":"en""#), "{detail}");
}

#[tokio::test]
async fn sharing_decides_who_reaches_every_meeting_route() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let mut cookies = vec![];
    for e in ["amy@example.com", "bob@example.com", "cat@example.com"] {
        control
            .execute(
                "INSERT INTO invites (email, tenant_id, role) VALUES (?1, ?2, 'member')",
                rusqlite::params![e, tid],
            )
            .unwrap();
        cookies.push(send(&app, login(e)).await.1.unwrap());
    }
    let (amy, bob, cat) = (&cookies[0], &cookies[1], &cookies[2]);
    for c in [&boss, amy, bob, cat] {
        send(&app, get("/api/meetings", c)).await;
    }
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let amy_id: String = t
        .query_row(
            "SELECT id FROM members WHERE email = 'amy@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let id = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "ready");
    // Bob is invited (upper case on the invite), cat is not.
    t.execute(
        "INSERT INTO calendar_events (id, member_id, external_id, title, starts_at, ends_at, meeting_id, attendees)
         VALUES ('e1', ?1, 'x', 'Sync', 0, 1, ?2, '[{\"email\":\"BOB@example.com\"}]')",
        rusqlite::params![amy_id, id],
    )
    .unwrap();
    let routes = [
        format!("/api/meetings/{id}"),
        format!("/api/meetings/{id}/transcript"),
        format!("/api/meetings/{id}/report"),
        format!("/api/meetings/{id}/audio"),
    ];
    let reach = |who: &str| {
        let who = who.to_string();
        let (app, routes, id) = (app.clone(), routes.clone(), id.clone());
        async move {
            let mut codes = vec![];
            for r in &routes {
                codes.push(send(&app, get(r, &who)).await.0);
            }
            let (_, _, list) = send(&app, get("/api/meetings", &who)).await;
            (codes, list.contains(&id))
        }
    };
    let ok = vec![StatusCode::OK; 4];
    let nf = vec![StatusCode::NOT_FOUND; 4];
    // Default `attendees`: the owner and the invited member, nobody else (not even the admin).
    assert_eq!(reach(amy).await, (ok.clone(), true));
    assert_eq!(reach(bob).await, (ok.clone(), true));
    assert_eq!(reach(cat).await, (nf.clone(), false));
    assert_eq!(reach(&boss).await, (nf.clone(), false));

    let patch = |c: &str, b: &str| json_req("PATCH", &format!("/api/meetings/{id}"), c, b);
    let share = || -> String {
        t.query_row("SELECT share FROM meetings WHERE id = ?1", [&id], |r| {
            r.get(0)
        })
        .unwrap()
    };
    // An attendee may not change it; a bad value is rejected; nothing moved.
    let (s, ..) = send(&app, patch(bob, r#"{"share":"workspace"}"#)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, ..) = send(&app, patch(cat, r#"{"share":"workspace"}"#)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, ..) = send(&app, patch(amy, r#"{"share":"everyone"}"#)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, ..) = send(&app, patch(amy, "{}")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(share(), "attendees");

    // Only me: the attendee loses it.
    let (s, _, body) = send(&app, patch(amy, r#"{"share":"owner"}"#)).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, r#"{"share":"owner"}"#));
    assert_eq!(reach(bob).await, (nf.clone(), false));
    assert_eq!(reach(amy).await, (ok.clone(), true));
    // Whole team: everyone sees it; the admin may then change it back.
    send(&app, patch(amy, r#"{"share":"workspace"}"#)).await;
    assert_eq!(reach(cat).await, (ok.clone(), true));
    let (_, _, detail) = send(&app, get(&format!("/api/meetings/{id}"), cat)).await;
    assert!(
        detail.contains(r#""share":"workspace","can_edit":false"#),
        "{detail}"
    );
    let (s, ..) = send(&app, patch(&boss, r#"{"share":"attendees"}"#)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(share(), "attendees");
}

#[tokio::test]
async fn action_items_are_edited_by_members_who_see_the_meeting() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (_, boss, _) = send(&app, login("boss@example.com")).await;
    let boss = boss.unwrap();
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    for e in ["amy@example.com", "bob@example.com"] {
        control
            .execute(
                "INSERT INTO invites (email, tenant_id, role) VALUES (?1, ?2, 'member')",
                rusqlite::params![e, tid],
            )
            .unwrap();
    }
    let (_, amy, _) = send(&app, login("amy@example.com")).await;
    let amy = amy.unwrap();
    let (_, bob, _) = send(&app, login("bob@example.com")).await;
    let bob = bob.unwrap();
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    for c in [&boss, &amy, &bob] {
        send(&app, get("/api/meetings", c)).await;
    }
    let mid = |e: &str| -> String {
        t.query_row("SELECT id FROM members WHERE email=?1", [e], |r| r.get(0))
            .unwrap()
    };
    let (amy_id, bob_id) = (mid("amy@example.com"), mid("bob@example.com"));
    let m = seed_full_meeting(&t, dir.path(), &tid, &amy_id, "ready");
    t.execute_batch(&format!(
        "INSERT OR REPLACE INTO reports (meeting_id, summary) VALUES ('{m}', 'S');
         INSERT INTO action_items (id, meeting_id, text) VALUES ('ai1', '{m}', 'write');"
    ))
    .unwrap();
    let row = || -> (i64, Option<String>, Option<String>) {
        t.query_row(
            "SELECT done, owner_member_id, due_date FROM action_items WHERE id='ai1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    };
    let patch = |c: &str, b: &str| json_req("PATCH", "/api/action-items/ai1", c, b);

    let (s, ..) = send(&app, patch(&bob, r#"{"done":true}"#)).await;
    assert_eq!(
        (s, row().0),
        (StatusCode::NOT_FOUND, 0),
        "bob cannot see it"
    );
    let (s, ..) = send(
        &app,
        patch(&amy, r#"{"done":true,"due_date":"2026-12-01"}"#),
    )
    .await;
    assert_eq!(
        (s, row()),
        (StatusCode::OK, (1, None, Some("2026-12-01".into())))
    );
    let own_bob = format!(r#"{{"owner_id":"{bob_id}"}}"#);
    let (s, ..) = send(&app, patch(&amy, &own_bob)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "owner must see the meeting");
    t.execute("UPDATE meetings SET share='workspace'", [])
        .unwrap();
    let (s, ..) = send(&app, patch(&amy, &own_bob)).await;
    assert_eq!(
        (s, row()),
        (
            StatusCode::OK,
            (1, Some(bob_id.clone()), Some("2026-12-01".into()))
        ),
        "done and due are kept when only the owner is sent"
    );
    let (_, _, body) = send(&app, get(&format!("/api/meetings/{m}/report"), &bob)).await;
    let r: serde_json::Value = serde_json::from_str(&body).unwrap();
    let ai = r["action_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "ai1")
        .unwrap();
    assert_eq!(ai["owner_id"], bob_id.as_str());
    assert_eq!(r["members"].as_array().unwrap().len(), 3);
    let (s, ..) = send(&app, patch(&amy, r#"{"owner_id":null,"due_date":null}"#)).await;
    assert_eq!((s, row()), (StatusCode::OK, (1, None, None)));
    let (s, ..) = send(&app, patch(&amy, r#"{"due_date":"soon"}"#)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let no_csrf = Request::patch("/api/action-items/ai1")
        .header(header::COOKIE, format!("{amy}; recapz_csrf=t"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"done":false}"#))
        .unwrap();
    let (s, ..) = send(&app, no_csrf).await;
    assert_eq!((s, row().0), (StatusCode::FORBIDDEN, 1), "csrf required");
}

#[tokio::test]
async fn purging_a_participant_is_admin_only_csrf_protected_and_removes_their_words_everywhere() {
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
    send(&app, upload(&boss, "Weekly sync")).await;
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let id: String = t
        .query_row("SELECT id FROM meetings", [], |r| r.get(0))
        .unwrap();
    t.execute_batch(&format!(
        "UPDATE meetings SET status='ready' WHERE id='{id}';
         INSERT INTO speakers (id, meeting_id, label, display_name) VALUES
           ('s1','{id}','Speaker 1','Lan'), ('s2','{id}','Speaker 2','Minh');
         INSERT INTO segments (meeting_id, speaker_id, start_ms, end_ms, text) VALUES
           ('{id}','s1',0,1000,'zebrafish budget'), ('{id}','s2',1000,2000,'walrus plan');
         INSERT INTO reports (meeting_id, summary) VALUES ('{id}','zebrafish summary');
         INSERT INTO chunks (meeting_id, start_ms, end_ms, text, embedding)
           VALUES ('{id}',0,2000,'zebrafish budget walrus plan',x'00');"
    ))
    .unwrap();
    let fts = || {
        count(
            &t,
            "SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'zebrafish'",
        )
    };
    assert_eq!(fts(), 1, "seed must be searchable");

    let (s, _, b) = send(&app, get("/api/participants", &boss)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&b).unwrap(),
        json!([{"name":"Lan","meetings":1},{"name":"Minh","meetings":1}])
    );
    let (s, ..) = send(&app, get("/api/participants", &amy)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let body = r#"{"name":"Lan"}"#;
    let purge =
        |cookie: &str, body: &str| json_req("POST", "/api/participants/purge", cookie, body);
    let (s, ..) = send(&app, purge(&amy, body)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "members cannot purge");
    let no_csrf = Request::post("/api/participants/purge")
        .header(header::COOKIE, boss.as_str())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    let (s, ..) = send(&app, no_csrf).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "csrf required");
    let (s, ..) = send(&app, purge(&boss, r#"{"name":" "}"#)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(
        count(&t, "SELECT count(*) FROM segments"),
        2,
        "refusals delete nothing"
    );
    assert_eq!(fts(), 1);

    let (s, _, b) = send(&app, purge(&boss, body)).await;
    assert_eq!((s, b.as_str()), (StatusCode::OK, r#"{"meetings":1}"#));

    let (_, _, b) = send(&app, get(&format!("/api/meetings/{id}/transcript"), &boss)).await;
    assert!(!b.contains("zebrafish") && b.contains("walrus plan"), "{b}");
    let (_, _, b) = send(&app, get(&format!("/api/meetings/{id}/report"), &boss)).await;
    assert!(!b.contains("zebrafish"), "{b}");
    assert_eq!(fts(), 0, "FTS");
    let hits =
        recapz_core::search::search(&t, "any", "zebrafish", &recapz_core::embed::HashEmbedder, 5)
            .unwrap();
    assert!(hits.is_empty(), "Ask");
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM jobs WHERE state='queued' AND kind IN ('index','report')"
        ),
        2
    );
    let (_, _, b) = send(&app, get("/api/participants", &boss)).await;
    assert_eq!(b, r#"[{"name":"Minh","meetings":1}]"#);
}
