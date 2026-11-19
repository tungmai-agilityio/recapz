//! Browser recording over a real socket: the start route, the refusals, and a call recorded, stopped,
//! discarded and reconnected end to end (no speech provider: the live transcript is off without a key).
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use recapz_core::db;
use recapz_server::{config::Config, routes};
use std::{
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, protocol::frame::coding::CloseCode, Error, Message,
};
use tower::ServiceExt;

const ORIGIN: &str = "http://localhost:8080";

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The app served on a free port, plus the router for plain requests.
async fn serve(dir: &Path) -> (SocketAddr, Router) {
    let cfg = Config {
        data_dir: dir.into(),
        dev_login: true,
        base_url: ORIGIN.into(),
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
    let app = routes::router_with_llm(cfg, Arc::new(Mutex::new(control)), None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(std::future::IntoFuture::into_future(axum::serve(
        listener,
        app.clone(),
    )));
    (addr, app)
}

fn tenant(dir: &Path) -> rusqlite::Connection {
    let control = db::open_control(dir).unwrap();
    db::open_tenant(dir, &db::tenant_ids(&control).unwrap().remove(0)).unwrap()
}

async fn login(app: &Router, email: &str) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::post("/auth/dev-login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(r#"{{"email":"{email}"}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .find(|c| c.starts_with("recapz_session="))
        .unwrap()
}

async fn start(app: &Router, cookie: &str, body: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(
            Request::post("/api/recordings")
                .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
                .header("x-csrf-token", "t")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let b = res.into_body().collect().await.unwrap().to_bytes();
    let id = serde_json::from_slice::<serde_json::Value>(&b)
        .ok()
        .and_then(|v| v["id"].as_str().map(String::from))
        .unwrap_or_default();
    (status, id)
}

async fn record(app: &Router, cookie: &str) -> String {
    let (status, id) = start(
        app,
        cookie,
        r#"{"title":"Weekly sync","language":"en","mode":"tab"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    id
}

/// Opens the recording socket, or the HTTP status it was refused with.
async fn connect(
    addr: SocketAddr,
    id: &str,
    cookie: &str,
    origin: Option<&str>,
) -> Result<Ws, StatusCode> {
    let mut req = format!("ws://{addr}/api/meetings/{id}/record")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    if let Some(o) = origin {
        req.headers_mut().insert(header::ORIGIN, o.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => Ok(ws),
        Err(Error::Http(res)) => Err(res.status()),
        Err(e) => panic!("connect failed: {e}"),
    }
}

/// Connects again after a socket closed, giving the server a moment to let the old one go.
async fn reconnect(addr: SocketAddr, id: &str, cookie: &str) -> Ws {
    for _ in 0..50 {
        match connect(addr, id, cookie, Some(ORIGIN)).await {
            Ok(ws) => return ws,
            Err(StatusCode::CONFLICT) => tokio::time::sleep(Duration::from_millis(20)).await,
            Err(s) => panic!("reconnect refused with {s}"),
        }
    }
    panic!("the old socket was never released")
}

fn frame(v: i16) -> Message {
    let bytes: Vec<u8> = std::iter::repeat_n(v.to_le_bytes(), 1600)
        .flatten()
        .collect();
    Message::Binary(bytes.into())
}

async fn send_frames(ws: &mut Ws, n: usize) {
    for i in 0..n {
        ws.send(frame(i as i16)).await.unwrap();
    }
}

/// The meeting's status once its call has ended, or `None` when the row is gone.
async fn settled(dir: &Path, id: &str) -> Option<String> {
    let c = tenant(dir);
    for _ in 0..250 {
        let s: Option<String> = c
            .query_row("SELECT status FROM meetings WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .ok();
        if s.as_deref() != Some("recording") {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the call never ended")
}

fn wav_len(dir: &Path, id: &str) -> u64 {
    let control = db::open_control(dir).unwrap();
    let t = db::tenant_ids(&control).unwrap().remove(0);
    std::fs::metadata(
        db::tenant_dir(dir, &t)
            .join("audio")
            .join(id)
            .join("recording.wav"),
    )
    .unwrap()
    .len()
}

#[tokio::test]
async fn start_checks_its_input() {
    let dir = tempfile::tempdir().unwrap();
    let (_, app) = serve(dir.path()).await;
    let boss = login(&app, "boss@example.com").await;
    for body in [
        r#"{"title":"x","mode":"screen"}"#,
        r#"{"title":"  ","mode":"tab"}"#,
        r#"{"title":"x","language":"fr","mode":"room"}"#,
    ] {
        assert_eq!(
            start(&app, &boss, body).await.0,
            StatusCode::BAD_REQUEST,
            "{body}"
        );
    }
    let (status, _) = start(&app, "recapz_session=bad", r#"{"title":"x","mode":"tab"}"#).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let n: i64 = tenant(dir.path())
        .query_row("SELECT count(*) FROM meetings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_recording_is_streamed_stopped_and_handed_to_transcription() {
    let dir = tempfile::tempdir().unwrap();
    let (addr, app) = serve(dir.path()).await;
    let boss = login(&app, "boss@example.com").await;
    let id = record(&app, &boss).await;
    let (owner, platform, language, title): (String, String, String, String) = tenant(dir.path())
        .query_row(
            "SELECT m.email, t.platform, t.language, t.title FROM meetings t JOIN members m ON m.id = t.owner_id WHERE t.id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (
            owner.as_str(),
            platform.as_str(),
            language.as_str(),
            title.as_str()
        ),
        ("boss@example.com", "browser", "en", "Weekly sync")
    );

    let mut ws = connect(addr, &id, &boss, Some(ORIGIN)).await.unwrap();
    send_frames(&mut ws, 12).await;
    ws.send(Message::Text("stop".into())).await.unwrap();
    assert_eq!(
        settled(dir.path(), &id).await.as_deref(),
        Some("transcribing")
    );
    assert_eq!(
        wav_len(dir.path(), &id),
        44 + 12 * 3200,
        "every frame was written"
    );
    let jobs: i64 = tenant(dir.path())
        .query_row(
            "SELECT count(*) FROM jobs WHERE kind='transcribe' AND meeting_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(jobs, 1);
    // The call is over: the socket is refused now.
    assert_eq!(
        connect(addr, &id, &boss, Some(ORIGIN)).await.err(),
        Some(StatusCode::CONFLICT)
    );
}

#[tokio::test]
async fn discard_deletes_the_meeting_and_its_audio() {
    let dir = tempfile::tempdir().unwrap();
    let (addr, app) = serve(dir.path()).await;
    let boss = login(&app, "boss@example.com").await;
    let id = record(&app, &boss).await;
    let mut ws = connect(addr, &id, &boss, Some(ORIGIN)).await.unwrap();
    send_frames(&mut ws, 5).await;
    ws.send(Message::Text("discard".into())).await.unwrap();
    assert_eq!(settled(dir.path(), &id).await, None, "the row is gone");
    let control = db::open_control(dir.path()).unwrap();
    let t = db::tenant_ids(&control).unwrap().remove(0);
    assert!(!db::tenant_dir(dir.path(), &t)
        .join("audio")
        .join(&id)
        .exists());
}

#[tokio::test]
async fn a_reconnect_within_the_quiet_limit_continues_the_same_recording() {
    let dir = tempfile::tempdir().unwrap();
    let (addr, app) = serve(dir.path()).await;
    let boss = login(&app, "boss@example.com").await;
    let id = record(&app, &boss).await;
    let mut ws = connect(addr, &id, &boss, Some(ORIGIN)).await.unwrap();
    send_frames(&mut ws, 5).await;
    ws.close(None).await.unwrap();
    drop(ws);
    let mut ws = reconnect(addr, &id, &boss).await;
    send_frames(&mut ws, 7).await;
    ws.send(Message::Text("stop".into())).await.unwrap();
    assert_eq!(
        settled(dir.path(), &id).await.as_deref(),
        Some("transcribing")
    );
    assert_eq!(
        wav_len(dir.path(), &id),
        44 + 12 * 3200,
        "both sockets' frames, one file"
    );
}

#[tokio::test]
async fn the_socket_refuses_strangers_second_sockets_finished_meetings_and_foreign_origins() {
    let dir = tempfile::tempdir().unwrap();
    let (addr, app) = serve(dir.path()).await;
    let boss = login(&app, "boss@example.com").await;
    let amy = login(&app, "amy@example.com").await;
    let id = record(&app, &boss).await;
    let t = tenant(dir.path());
    // Shared with the workspace, so amy can see it, but only the owner records.
    t.execute("UPDATE meetings SET share='workspace' WHERE id=?1", [&id])
        .unwrap();
    assert_eq!(
        connect(addr, &id, &amy, Some(ORIGIN)).await.err(),
        Some(StatusCode::NOT_FOUND)
    );
    for origin in [
        Some("http://evil.example"),
        Some("http://localhost:8080.evil"),
        None,
    ] {
        assert_eq!(
            connect(addr, &id, &boss, origin).await.err(),
            Some(StatusCode::FORBIDDEN),
            "{origin:?}"
        );
    }
    assert_eq!(
        connect(addr, &id, "recapz_session=bad", Some(ORIGIN))
            .await
            .err(),
        Some(StatusCode::UNAUTHORIZED)
    );

    let _first = connect(addr, &id, &boss, Some(ORIGIN)).await.unwrap();
    assert_eq!(
        connect(addr, &id, &boss, Some(ORIGIN)).await.err(),
        Some(StatusCode::CONFLICT),
        "a second socket"
    );

    // A meeting of the owner's that is not a running browser recording.
    t.execute(
        "INSERT INTO meetings (id, title, platform, status, owner_id) SELECT 'up', 'u', 'upload', 'ready', owner_id FROM meetings WHERE id=?1",
        [&id],
    )
    .unwrap();
    t.execute(
        "INSERT INTO meetings (id, title, platform, status, owner_id) SELECT 'stale', 's', 'browser', 'recording', owner_id FROM meetings WHERE id=?1",
        [&id],
    )
    .unwrap();
    for (mid, want) in [
        ("up", StatusCode::CONFLICT),
        ("stale", StatusCode::CONFLICT), // recording on paper, but no call runs (a restart)
        ("nope", StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            connect(addr, mid, &boss, Some(ORIGIN)).await.err(),
            Some(want),
            "{mid}"
        );
    }
}

#[tokio::test]
async fn anything_but_a_whole_frame_or_a_command_closes_the_socket_and_keeps_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let (addr, app) = serve(dir.path()).await;
    let boss = login(&app, "boss@example.com").await;
    let id = record(&app, &boss).await;
    for bad in [
        Message::Binary(vec![0u8; 3198].into()),
        Message::Binary(vec![0u8; 3202].into()),
        Message::Text("pause".into()),
    ] {
        let mut ws = reconnect(addr, &id, &boss).await;
        ws.send(bad).await.unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("the server kept the socket open");
        let Some(Ok(Message::Close(Some(f)))) = reply else {
            panic!("expected a close frame, got {reply:?}");
        };
        assert_eq!(f.code, CloseCode::Policy);
    }
    let n: i64 = tenant(dir.path())
        .query_row(
            "SELECT count(*) FROM meetings WHERE id=?1 AND status='recording'",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "the call waits for a reconnect");
    let mut ws = reconnect(addr, &id, &boss).await;
    ws.send(Message::Text("stop".into())).await.unwrap();
    // No audio arrived, so the call fails rather than transcribing nothing.
    assert_eq!(settled(dir.path(), &id).await.as_deref(), Some("failed"));
}
