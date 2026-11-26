//! One person in two workspaces: every route that takes a meeting, document or calendar event id answers
//! 404 for an id of the other workspace and changes nothing, and Ask / Live Ask never cite it.
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use recapz_core::{
    db,
    embed::{to_blob, Embedder, HashEmbedder},
};
use recapz_llm::{stub::Stub, Price, Route, Router as Llm, Task};
use recapz_server::{
    config::Config,
    live::{hub, Msg},
    routes,
};
use rusqlite::params;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Error};
use tower::ServiceExt;

const ORIGIN: &str = "http://localhost:8080";
const OTHER_TEXT: &str = "the zebrafish retainer is sixty thousand";
const OWN_TEXT: &str = "the ocelot payroll runs monthly";

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

fn req(method: &str, uri: &str, cookie: &str, body: Option<&str>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t")
        .body(Body::from(body.unwrap_or("{}").to_owned()))
        .unwrap()
}

struct Fixture {
    app: Router,
    addr: SocketAddr,
    dir: tempfile::TempDir,
    stub: Arc<Stub>,
    /// Boss signed in; the session is in the first workspace.
    boss: String,
    /// The second workspace, which owns every `OTHER_*` row.
    second: String,
}

/// Boss belongs to both workspaces and is in the first. The second holds a recording browser meeting
/// (live), a speaker, a segment, a report, a chunk, a document and a calendar event, all visible to boss
/// had he been in that workspace. The first holds one meeting and one document of its own.
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let control = db::open_control(dir.path()).unwrap();
    db::bootstrap_admin(&control, "boss@example.com").unwrap();
    let first = db::tenant_ids(&control).unwrap().remove(0);
    let second = db::create_workspace(&control, dir.path(), "Second", "boss@example.com").unwrap();
    let cfg = Config {
        data_dir: dir.path().into(),
        dev_login: true,
        base_url: ORIGIN.into(),
        ..Config::default()
    };
    let stub = Arc::new(Stub::texts(&[
        r#"{"found":true,"answer":"Sixty.","citations":[1]}"#,
        r#"{"found":true,"answer":"Sixty.","citations":[1]}"#,
        r#"{"found":true,"answer":"Sixty.","citations":[1]}"#,
    ]));
    let route = Route {
        provider: "p".into(),
        model: "m".into(),
        max_tokens: 10,
        price: Price {
            input: 1_000_000,
            output: 0,
        },
    };
    let llm = Arc::new(
        Llm::new()
            .provider("p", stub.clone())
            .route(Task::LiveAnswer, route.clone())
            .route(Task::AskAnswer, route),
    );
    let app = routes::router_with_llm(cfg, Arc::new(Mutex::new(control)), Some(llm));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(std::future::IntoFuture::into_future(axum::serve(
        listener,
        app.clone(),
    )));
    let login = Request::post("/auth/dev-login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"email":"boss@example.com"}"#))
        .unwrap();
    let boss = send(&app, login).await.1.unwrap();

    let control = db::open_control(dir.path()).unwrap();
    let uid: String = control
        .query_row(
            "SELECT id FROM users WHERE email='boss@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for (tid, own) in [(&first, true), (&second, false)] {
        let t = db::open_tenant(dir.path(), tid).unwrap();
        t.execute(
            "INSERT INTO members (id, email, role) VALUES (?1, 'boss@example.com', 'admin') ON CONFLICT DO NOTHING",
            [&uid],
        )
        .unwrap();
        let (m, d, text) = if own {
            ("m1", "d1", OWN_TEXT)
        } else {
            ("m2", "d2", OTHER_TEXT)
        };
        t.execute(
            "INSERT INTO meetings (id, title, platform, status, owner_id, share) VALUES (?1, 'Title', 'browser', 'recording', ?2, 'workspace')",
            params![m, uid],
        )
        .unwrap();
        t.execute(
            "INSERT INTO chunks (meeting_id, start_ms, end_ms, text, embedding) VALUES (?1, 0, 1000, ?2, ?3)",
            params![m, text, to_blob(&HashEmbedder.embed(text).unwrap())],
        )
        .unwrap();
        t.execute(
            "INSERT INTO documents (id, member_id, name, kind, size_bytes, path, llm_ok, created_at) VALUES (?1, ?2, 'doc.md', 'md', 1, 'p', 1, 0)",
            params![d, uid],
        )
        .unwrap();
        t.execute(
            "INSERT INTO document_chunks (document_id, seq, text) VALUES (?1, 0, ?2)",
            params![d, text],
        )
        .unwrap();
        t.execute(
            "INSERT INTO document_vectors (chunk_id, document_id, embedder, embedding) VALUES (last_insert_rowid(), ?1, ?2, ?3)",
            params![d, HashEmbedder.id(), to_blob(&HashEmbedder.embed(text).unwrap())],
        )
        .unwrap();
        if !own {
            t.execute(
                "INSERT INTO speakers (id, meeting_id, label) VALUES ('s2', 'm2', 'Speaker 1')",
                [],
            )
            .unwrap();
            t.execute(
                "INSERT INTO segments (meeting_id, speaker_id, start_ms, end_ms, text) VALUES ('m2', 's2', 0, 1000, ?1)",
                [text],
            )
            .unwrap();
            // A real recording, so the audio route's 404 can only come from the workspace boundary.
            t.execute(
                "UPDATE meetings SET audio_path = 'a.wav' WHERE id = 'm2'",
                [],
            )
            .unwrap();
            let dir = db::tenant_dir(dir.path(), tid);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("a.wav"), b"RIFF").unwrap();
            t.execute(
                "INSERT INTO calendar_events (id, member_id, external_id, title, starts_at, ends_at) VALUES ('e2', ?1, 'x', 'Title', 0, 1)",
                [&uid],
            )
            .unwrap();
        }
    }
    hub().start("m1");
    hub().start("m2");
    for (m, text) in [("m1", OWN_TEXT), ("m2", OTHER_TEXT)] {
        hub().push(
            m,
            Msg::Final {
                at_ms: 0,
                text: text.into(),
            },
        );
    }
    Fixture {
        app,
        addr,
        dir,
        stub,
        boss,
        second,
    }
}

impl Fixture {
    /// Every row the routes below could touch in the second workspace.
    fn second_state(&self) -> String {
        let t = db::open_tenant(self.dir.path(), &self.second).unwrap();
        let mut out = Vec::new();
        for sql in [
            "SELECT id||title||language||status||share FROM meetings",
            "SELECT id||llm_ok FROM documents",
            "SELECT id||auto_join FROM calendar_events",
            "SELECT id||label FROM speakers",
            "SELECT text FROM segments",
            "SELECT text FROM chunks",
            "SELECT kind||ifnull(meeting_id,'') FROM jobs",
        ] {
            let mut st = t.prepare(sql).unwrap();
            out.extend(
                st.query_map([], |r| r.get::<_, String>(0))
                    .unwrap()
                    .map(Result::unwrap),
            );
        }
        out.join("|")
    }

    async fn record_status(&self, id: &str) -> Result<(), StatusCode> {
        let mut r = format!("ws://{}/api/meetings/{id}/record", self.addr)
            .into_client_request()
            .unwrap();
        r.headers_mut()
            .insert(header::COOKIE, self.boss.parse().unwrap());
        r.headers_mut()
            .insert(header::ORIGIN, ORIGIN.parse().unwrap());
        match tokio_tungstenite::connect_async(r).await {
            Ok(_) => Ok(()),
            Err(Error::Http(res)) => Err(res.status()),
            Err(e) => panic!("connect failed: {e}"),
        }
    }
}

#[tokio::test]
async fn every_id_route_answers_404_for_the_other_workspace_and_changes_nothing() {
    let f = fixture().await;
    let before = f.second_state();
    assert!(before.contains("m2Title"), "{before}");
    let routes: [(&str, &str, Option<&str>); 13] = [
        ("GET", "/api/meetings/m2", None),
        ("PATCH", "/api/meetings/m2", Some(r#"{"title":"hacked"}"#)),
        ("DELETE", "/api/meetings/m2", None),
        (
            "PATCH",
            "/api/meetings/m2/language",
            Some(r#"{"language":"en"}"#),
        ),
        ("GET", "/api/meetings/m2/transcript", None),
        ("GET", "/api/meetings/m2/report", None),
        ("GET", "/api/meetings/m2/audio", None),
        ("GET", "/api/meetings/m2/live", None),
        ("POST", "/api/meetings/m2/live-ask", Some("{}")),
        ("PATCH", "/api/documents/d2", Some(r#"{"llm_ok":false}"#)),
        ("DELETE", "/api/documents/d2", None),
        (
            "PATCH",
            "/api/calendar-events/e2",
            Some(r#"{"auto_join":true}"#),
        ),
        ("DELETE", "/api/dev/live-sim/m2", None),
    ];
    for (method, uri, body) in routes {
        // Status only: a leaked live view would be a stream that never ends.
        let res = f.app.clone().oneshot(req(method, uri, &f.boss, body)).await;
        let s = res.unwrap().status();
        // The dev simulator is off here, so it answers 404 for every id; the rest must find nothing.
        assert_eq!(s, StatusCode::NOT_FOUND, "{method} {uri}");
    }
    // The record socket is a plain GET with an upgrade, so it needs a real connection.
    assert_eq!(f.record_status("m2").await, Err(StatusCode::NOT_FOUND));
    assert_eq!(f.second_state(), before);
    assert!(f.stub.requests().is_empty());

    // Control: the same ids work from inside the second workspace, so the 404 above was the boundary.
    let switch = format!("/api/workspaces/{}/switch", f.second);
    let (s, ..) = send(&f.app, req("POST", &switch, &f.boss, None)).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    for uri in [
        "/api/meetings/m2",
        "/api/meetings/m2/transcript",
        "/api/meetings/m2/audio",
        "/api/meetings/m2/live",
    ] {
        // Status only: the live view is a stream that never ends.
        let res = f.app.clone().oneshot(req("GET", uri, &f.boss, None)).await;
        assert_eq!(res.unwrap().status(), StatusCode::OK, "{uri}");
    }
    // And the first workspace's meeting is now the foreign id.
    let (s, ..) = send(&f.app, req("GET", "/api/meetings/m1", &f.boss, None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ask_and_live_ask_never_cite_the_other_workspace() {
    let f = fixture().await;
    let ask = |q: &str| {
        let body = serde_json::json!({ "question": q }).to_string();
        req("POST", "/api/ask", &f.boss, Some(&body))
    };
    // Boss is in the first workspace: a question only the second can answer finds nothing, no model call.
    let (s, _, body) = send(&f.app, ask("how much is the zebrafish retainer?")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        !body.contains("m2") && !body.contains("d2") && !body.contains("sixty"),
        "{body}"
    );
    assert!(f.stub.requests().is_empty());

    // Live Ask in the first workspace's own call sends only its own words, never the other call's.
    let (s, _, body) = send(
        &f.app,
        req("POST", "/api/meetings/m1/live-ask", &f.boss, None),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let sent = f.stub.requests();
    assert_eq!(sent.len(), 1);
    assert!(!sent[0].prompt.contains("zebrafish"), "{}", sent[0].prompt);
    assert!(!sent[0].prompt.contains("sixty"), "{}", sent[0].prompt);
    assert!(!body.contains("m2") && !body.contains("d2"), "{body}");

    // Control: after switching, the same question reaches the second workspace's meeting and document.
    let switch = format!("/api/workspaces/{}/switch", f.second);
    send(&f.app, req("POST", &switch, &f.boss, None)).await;
    let (s, _, body) = send(&f.app, ask("how much is the zebrafish retainer?")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains(r#""meeting_id":"m2""#), "{body}");
    assert!(!body.contains("m1"), "{body}");
}
