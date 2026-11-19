use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use recapz_core::db;
use recapz_llm::{stub::Stub, Price, Route, Router as Llm, Task};
use recapz_server::{config::Config, live::hub, routes};
use rusqlite::params;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

fn app(dir: &std::path::Path, dev: bool, stub: Option<Arc<Stub>>) -> Router {
    app_keyed(dir, dev, stub, true)
}

fn app_keyed(dir: &std::path::Path, dev: bool, stub: Option<Arc<Stub>>, keyed: bool) -> Router {
    let cfg = Config {
        data_dir: dir.into(),
        dev_login: dev,
        speech: recapz_server::config::Speech {
            base_url: "http://127.0.0.1:1".into(),
            api_key: if keyed { "k".into() } else { String::new() },
            ..Default::default()
        },
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
    let llm = stub.map(|s| {
        Arc::new(Llm::new().provider("p", s).route(
            Task::LiveAnswer,
            Route {
                provider: "p".into(),
                model: "m".into(),
                max_tokens: 10,
                price: Price::default(),
            },
        ))
    });
    routes::router_with_llm(cfg, Arc::new(Mutex::new(control)), llm)
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Option<String>, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .find(|c| c.starts_with("recapz_session="));
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, cookie, String::from_utf8_lossy(&body).into())
}

async fn login(app: &Router, email: &str) -> String {
    send(
        app,
        Request::post("/auth/dev-login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(r#"{{"email":"{email}"}}"#)))
            .unwrap(),
    )
    .await
    .1
    .unwrap()
}

fn req(method: &str, uri: &str, cookie: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

fn id_of(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn status_of(dir: &std::path::Path, id: &str) -> Option<String> {
    let control = db::open_control(dir).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    db::open_tenant(dir, &tid)
        .unwrap()
        .query_row("SELECT status FROM meetings WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .ok()
}

#[tokio::test]
async fn the_simulator_is_refused_unless_dev_login_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), false, None);
    let (s, ..) = send(
        &app,
        req("POST", "/api/dev/live-sim", "recapz_session=x", "{}"),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, ..) = send(
        &app,
        req("DELETE", "/api/dev/live-sim/abc", "recapz_session=x", ""),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(hub().subscribe("abc").is_none());
}

#[tokio::test]
async fn a_simulated_call_feeds_the_live_view_and_live_ask_and_only_its_owner_can_use_or_stop_it() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(Stub::texts(&[
        r#"{"found":true,"answer":"Net 30.","citations":[1]}"#,
    ]));
    let app = app(dir.path(), true, Some(stub.clone()));
    let boss = login(&app, "boss@example.com").await;
    let amy = login(&app, "amy@example.com").await;
    // The boss's document that answers the simulated question.
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let user = |e: &str| -> String {
        control
            .query_row("SELECT id FROM users WHERE email=?1", [e], |r| r.get(0))
            .unwrap()
    };
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    for (id, e, role) in [
        (user("boss@example.com"), "boss@example.com", "admin"),
        (user("amy@example.com"), "amy@example.com", "member"),
    ] {
        t.execute(
            "INSERT INTO members (id, email, role) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
            params![id, e, role],
        )
        .unwrap();
    }
    t.execute(
        "INSERT INTO documents (id, member_id, name, kind, size_bytes, path, llm_ok, created_at) VALUES ('d1', ?1, 'rates.md', 'md', 1, 'p', 1, 0)",
        [user("boss@example.com")],
    )
    .unwrap();
    t.execute(
        "INSERT INTO document_chunks (document_id, seq, text) VALUES ('d1', 0, 'payment terms are net 30')",
        [],
    )
    .unwrap();

    let (s, _, b) = send(
        &app,
        req(
            "POST",
            "/api/dev/live-sim",
            &boss,
            r#"{"title":"Sim","lines":["what are the payment terms?","and the deposit?"],"interval_ms":200}"#,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = id_of(&b);
    assert_eq!(status_of(dir.path(), &id).as_deref(), Some("recording"));

    // The first line arrives in the live view.
    let res = app
        .clone()
        .oneshot(req("GET", &format!("/api/meetings/{id}/live"), &boss, ""))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body();
    let mut seen = String::new();
    while !seen.contains("event: final") {
        let f = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
            .await
            .expect("the simulated call said nothing")
            .expect("stream ended")
            .unwrap();
        seen.push_str(&String::from_utf8_lossy(&f.into_data().unwrap()));
    }
    assert!(seen.contains("what are the payment terms?"), "{seen}");

    // Ask now answers from the document, with the simulated words as the question.
    let (s, _, answer) = send(
        &app,
        req("POST", &format!("/api/meetings/{id}/live-ask"), &boss, "{}"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{answer}");
    assert!(answer.contains("Net 30."), "{answer}");
    assert!(answer.contains(r#""document_id":"d1""#), "{answer}");
    assert!(stub.requests()[0]
        .prompt
        .contains("what are the payment terms?"));

    // Another member cannot see it, ask about it or stop it; the meeting is private to the owner.
    for r in [
        req("GET", &format!("/api/meetings/{id}/live"), &amy, ""),
        req("POST", &format!("/api/meetings/{id}/live-ask"), &amy, "{}"),
        req("DELETE", &format!("/api/dev/live-sim/{id}"), &amy, ""),
    ] {
        assert_eq!(send(&app, r).await.0, StatusCode::NOT_FOUND);
    }

    // Stopping ends the meeting and clears the live transcript; a second stop finds nothing.
    let (s, ..) = send(
        &app,
        req("DELETE", &format!("/api/dev/live-sim/{id}"), &boss, ""),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(status_of(dir.path(), &id).as_deref(), Some("ready"));
    assert!(hub().subscribe(&id).is_none());
    let (s, ..) = send(
        &app,
        req("DELETE", &format!("/api/dev/live-sim/{id}"), &boss, ""),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn opting_out_deletes_the_simulated_meeting_and_a_real_one_is_never_touched() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), true, None);
    let boss = login(&app, "boss@example.com").await;
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let bid: String = control
        .query_row(
            "SELECT id FROM users WHERE email='boss@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    t.execute("INSERT INTO members (id, email, role) VALUES (?1, 'boss@example.com', 'admin') ON CONFLICT DO NOTHING", [&bid]).unwrap();
    // A real recording meeting of the boss (no simulator marker).
    t.execute(
        "INSERT INTO meetings (id, title, platform, join_url, status, owner_id) VALUES ('real1', 'Real', 'meet', 'https://meet.example/abc', 'recording', ?1)",
        [&bid],
    )
    .unwrap();

    let (_, _, b) = send(
        &app,
        req("POST", "/api/dev/live-sim", &boss, r#"{"fail_after":0}"#),
    )
    .await;
    let id = id_of(&b);
    let (s, ..) = send(
        &app,
        req(
            "DELETE",
            &format!("/api/dev/live-sim/{id}?optout=true"),
            &boss,
            "",
        ),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        status_of(dir.path(), &id),
        None,
        "opt-out deletes the meeting"
    );
    assert!(hub().subscribe(&id).is_none());

    let (s, ..) = send(&app, req("DELETE", "/api/dev/live-sim/real1", &boss, "")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(status_of(dir.path(), "real1").as_deref(), Some("recording"));
}

fn tone(seconds: usize) -> Vec<u8> {
    let n = seconds * 16_000;
    let samples: Vec<f32> = (0..n).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
    recapz_speech::deepgram::encode_wav(&samples)
}

fn audio_req(cookie: &str, file: &[u8], name: &str, language: Option<&str>) -> Request<Body> {
    let b = "XB";
    let mut body = Vec::new();
    if let Some(l) = language {
        body.extend(
            format!("--{b}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n{l}\r\n")
                .bytes(),
        );
    }
    body.extend(format!("--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: audio/wav\r\n\r\n").bytes());
    body.extend(file);
    body.extend(format!("\r\n--{b}--\r\n").bytes());
    Request::post("/api/dev/live-sim/audio")
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={b}"),
        )
        .body(Body::from(body))
        .unwrap()
}

async fn wait_for(dir: &std::path::Path, id: &str, want: Option<&str>) {
    for _ in 0..100 {
        if status_of(dir, id).as_deref() == want {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("status stayed {:?}, wanted {want:?}", status_of(dir, id));
}

fn transcribe_jobs(dir: &std::path::Path, id: &str) -> i64 {
    let control = db::open_control(dir).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    db::open_tenant(dir, &tid)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM jobs WHERE kind='transcribe' AND meeting_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn a_replayed_recording_plays_in_real_time_then_is_transcribed_like_a_real_call() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), true, None);
    let boss = login(&app, "boss@example.com").await;
    let (s, _, b) = send(&app, audio_req(&boss, &tone(1), "call.wav", Some("en"))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = id_of(&b);
    assert!(
        matches!(
            status_of(dir.path(), &id).as_deref(),
            Some("joining" | "recording")
        ),
        "the call is live right after the upload"
    );
    wait_for(dir.path(), &id, Some("transcribing")).await;
    assert_eq!(transcribe_jobs(dir.path(), &id), 1);
    assert!(
        hub().subscribe(&id).is_none(),
        "the live transcript is dropped when the call ends"
    );
}

#[tokio::test]
async fn a_replayed_call_can_be_ended_early_or_opted_out_and_only_by_its_owner() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), true, None);
    let boss = login(&app, "boss@example.com").await;
    let amy = login(&app, "amy@example.com").await;

    let (_, _, b) = send(&app, audio_req(&boss, &tone(60), "a.wav", None)).await;
    let ended = id_of(&b);
    wait_for(dir.path(), &ended, Some("recording")).await;
    let (s, ..) = send(
        &app,
        req("DELETE", &format!("/api/dev/live-sim/{ended}"), &amy, ""),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "another member cannot stop it");
    assert_eq!(status_of(dir.path(), &ended).as_deref(), Some("recording"));
    let (s, ..) = send(
        &app,
        req("DELETE", &format!("/api/dev/live-sim/{ended}"), &boss, ""),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    wait_for(dir.path(), &ended, Some("transcribing")).await;
    assert_eq!(transcribe_jobs(dir.path(), &ended), 1);

    let (_, _, b) = send(&app, audio_req(&boss, &tone(60), "b.wav", None)).await;
    let opted = id_of(&b);
    wait_for(dir.path(), &opted, Some("recording")).await;
    let (s, ..) = send(
        &app,
        req(
            "DELETE",
            &format!("/api/dev/live-sim/{opted}?optout=true"),
            &boss,
            "",
        ),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    wait_for(dir.path(), &opted, None).await;
    assert_eq!(
        transcribe_jobs(dir.path(), &opted),
        0,
        "opt-out leaves nothing to transcribe"
    );
}

#[tokio::test]
async fn a_replay_is_refused_without_dev_login_a_speech_key_or_readable_audio() {
    let dir = tempfile::tempdir().unwrap();
    let off = app(dir.path(), false, None);
    let (s, ..) = send(&off, audio_req("recapz_session=x", &tone(1), "a.wav", None)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let dir = tempfile::tempdir().unwrap();
    let nokey = app_keyed(dir.path(), true, None, false);
    let boss = login(&nokey, "boss@example.com").await;
    let (s, _, b) = send(&nokey, audio_req(&boss, &tone(1), "a.wav", None)).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");

    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), true, None);
    let boss = login(&app, "boss@example.com").await;
    let (s, ..) = send(&app, audio_req(&boss, b"not audio at all", "a.wav", None)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, ..) = send(&app, audio_req(&boss, &tone(1), "a.wav", Some("klingon"))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let control = db::open_control(dir.path()).unwrap();
    let tid = db::tenant_ids(&control).unwrap().remove(0);
    let n: i64 = db::open_tenant(dir.path(), &tid)
        .unwrap()
        .query_row("SELECT count(*) FROM meetings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "a refused upload creates no meeting");
}
