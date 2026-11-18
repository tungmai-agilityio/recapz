//! Recordings in a private object store, against a mock S3 server on localhost: archive, ranged
//! playback behind the meeting access rule, and hard delete of the object.
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use recapz_core::{archive_job, db, jobs};
use recapz_server::{config::Config, routes, storage::S3Store};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};
use tower::ServiceExt;

type Objects = Arc<Mutex<HashMap<String, Vec<u8>>>>;
/// Parts of in-flight multipart uploads, keyed by `key#part`.
type Parts = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// Path-style S3: `/bucket/key`. Supports PUT, HEAD, GET with `Range`, DELETE. Signatures are not checked.
fn mock_s3() -> (String, Objects) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let objects: Objects = Default::default();
    let (o, parts): (Objects, Parts) = (objects.clone(), Default::default());
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (o, p) = (o.clone(), parts.clone());
            thread::spawn(move || serve(s, o, p));
        }
    });
    (url, objects)
}

fn serve(mut s: TcpStream, objects: Objects, parts: Parts) {
    let mut raw = Vec::new();
    let mut buf = [0u8; 65536];
    let (head, mut body) = loop {
        let n = s.read(&mut buf).unwrap();
        if n == 0 {
            return;
        }
        raw.extend(&buf[..n]);
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break (
                String::from_utf8_lossy(&raw[..i]).to_string(),
                raw[i + 4..].to_vec(),
            );
        }
    };
    let mut lines = head.lines();
    let mut first = lines.next().unwrap().split(' ');
    let (method, path) = (first.next().unwrap(), first.next().unwrap());
    let query = path.split_once('?').map(|x| x.1).unwrap_or("").to_string();
    let q = |name: &str| {
        query
            .split('&')
            .find_map(|kv| kv.strip_prefix(&format!("{name}=")).map(str::to_string))
    };
    let key = path.split('?').next().unwrap().trim_start_matches('/');
    let key = key.split_once('/').map(|x| x.1).unwrap_or("").to_string(); // drop the bucket
    let header = |name: &str| {
        lines.clone().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    let want: usize = header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while body.len() < want {
        let n = s.read(&mut buf).unwrap();
        body.extend(&buf[..n]);
    }
    let reply = |s: &mut TcpStream, status: &str, extra: &str, len: usize, body: &[u8]| {
        write!(
            s,
            "HTTP/1.1 {status}\r\n{extra}Content-Length: {len}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        s.write_all(body).unwrap();
    };
    let mut o = objects.lock().unwrap();
    match method {
        "POST" if query.starts_with("uploads") => {
            let xml = format!("<InitiateMultipartUploadResult><Bucket>recordings</Bucket><Key>{key}</Key><UploadId>u1</UploadId></InitiateMultipartUploadResult>");
            reply(&mut s, "200 OK", "", xml.len(), xml.as_bytes());
        }
        "POST" => {
            // complete: stitch the parts in order
            let mut all = Vec::new();
            let mut parts = parts.lock().unwrap();
            for n in 1.. {
                match parts.remove(&format!("{key}#{n}")) {
                    Some(p) => all.extend(p),
                    None => break,
                }
            }
            o.insert(key, all);
            let xml = "<CompleteMultipartUploadResult><Location>l</Location><Bucket>b</Bucket><Key>k</Key><ETag>\"x\"</ETag></CompleteMultipartUploadResult>";
            reply(&mut s, "200 OK", "", xml.len(), xml.as_bytes());
        }
        "PUT" if q("partNumber").is_some() => {
            parts
                .lock()
                .unwrap()
                .insert(format!("{key}#{}", q("partNumber").unwrap()), body);
            reply(&mut s, "200 OK", "ETag: \"p\"\r\n", 0, b"");
        }
        "PUT" => {
            o.insert(key, body);
            reply(&mut s, "200 OK", "ETag: \"x\"\r\n", 0, b"");
        }
        "DELETE" => {
            o.remove(&key);
            reply(&mut s, "204 No Content", "", 0, b"");
        }
        "HEAD" | "GET" => match o.get(&key) {
            None => reply(&mut s, "404 Not Found", "", 0, b""),
            Some(d) if method == "HEAD" => write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                d.len()
            )
            .unwrap(),
            Some(d) => match header("range").and_then(|r| {
                let (a, b) = r.strip_prefix("bytes=")?.split_once('-')?;
                Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
            }) {
                Some((a, b)) => reply(
                    &mut s,
                    "206 Partial Content",
                    &format!("Content-Range: bytes {a}-{b}/{}\r\n", d.len()),
                    b - a + 1,
                    &d[a..=b],
                ),
                None => reply(&mut s, "200 OK", "", d.len(), d),
            },
        },
        _ => reply(&mut s, "405 Method Not Allowed", "", 0, b""),
    }
}

fn cfg(dir: &std::path::Path, endpoint: &str) -> Config {
    let mut c = Config {
        data_dir: dir.into(),
        dev_login: true,
        ..Config::default()
    };
    c.storage.endpoint = endpoint.into();
    c.storage.region = "test".into();
    c.storage.bucket = "recordings".into();
    c.storage.key_id = "id".into();
    c.storage.secret = "secret".into();
    c
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let res = app.clone().oneshot(req).await.unwrap();
    let (st, h) = (res.status(), res.headers().clone());
    (
        st,
        h,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

fn req(method: &str, path: &str, cookie: &str, range: Option<&str>) -> Request<Body> {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header(header::COOKIE, format!("{cookie}; recapz_csrf=t"))
        .header("x-csrf-token", "t");
    if let Some(range) = range {
        r = r.header(header::RANGE, range);
    }
    r.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn a_recording_is_archived_played_in_ranges_behind_access_and_deleted_from_the_bucket() {
    let dir = tempfile::tempdir().unwrap();
    let (url, objects) = mock_s3();
    let cfg = cfg(dir.path(), &url);
    let store = S3Store::from_config(&cfg.storage, tokio::runtime::Handle::current())
        .unwrap()
        .unwrap();
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
    let login = |email: &str| {
        Request::post("/auth/dev-login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(r#"{{"email":"{email}"}}"#)))
            .unwrap()
    };
    let cookie = |h: &axum::http::HeaderMap| {
        h.get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
            .find(|c| c.starts_with("recapz_session="))
            .unwrap()
    };
    let boss = cookie(&send(&app, login("boss@example.com")).await.1);
    let amy = cookie(&send(&app, login("amy@example.com")).await.1);
    send(&app, req("GET", "/api/meetings", &boss, None)).await; // creates the members row
    let (_, _, me) = send(&app, req("GET", "/api/me", &boss, None)).await;
    assert!(String::from_utf8_lossy(&me).contains(r#""recording_storage":"object_storage""#));

    // A finished meeting whose recording is 40 known bytes on local disk.
    let t = db::open_tenant(dir.path(), &tid).unwrap();
    let owner: String = t
        .query_row(
            "SELECT id FROM members WHERE email='boss@example.com'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let audio = db::tenant_dir(dir.path(), &tid).join("audio/m1");
    std::fs::create_dir_all(&audio).unwrap();
    let wav: Vec<u8> = (0..40u8).collect();
    std::fs::write(audio.join("recording.wav"), &wav).unwrap();
    t.execute(
        "INSERT INTO meetings (id, title, platform, status, owner_id, audio_path) VALUES ('m1','t','meet','ready',?1,'audio/m1/recording.wav')",
        [&owner],
    )
    .unwrap();
    jobs::enqueue(&t, archive_job::KIND, Some("m1"), None).unwrap();
    let job = jobs::claim(&t, &[archive_job::KIND]).unwrap().unwrap();
    // the job runs on a blocking thread in the server; the store blocks on the runtime
    let (d, tid2) = (dir.path().to_path_buf(), tid.clone());
    tokio::task::spawn_blocking(move || {
        archive_job::handle(&t, &job, &d, &tid2, &store).unwrap();
    })
    .await
    .unwrap();

    let key = format!("{tid}/audio/m1/recording.wav");
    assert_eq!(
        objects.lock().unwrap().get(&key),
        Some(&wav),
        "uploaded under tenant/path"
    );
    assert!(!audio.join("recording.wav").exists(), "local copy removed");

    let path = "/api/meetings/m1/audio";
    let (st, h, body) = send(&app, req("GET", path, &boss, None)).await;
    assert_eq!((st, body), (StatusCode::OK, wav.clone()));
    assert_eq!(h[header::ACCEPT_RANGES], "bytes");
    let (st, h, body) = send(&app, req("GET", path, &boss, Some("bytes=10-14"))).await;
    assert_eq!(st, StatusCode::PARTIAL_CONTENT);
    assert_eq!(h[header::CONTENT_RANGE], "bytes 10-14/40");
    assert_eq!(body, wav[10..=14].to_vec());
    let (_, h, body) = send(&app, req("GET", path, &boss, Some("bytes=-4"))).await;
    assert_eq!(
        (h[header::CONTENT_RANGE].to_str().unwrap(), body),
        ("bytes 36-39/40", wav[36..].to_vec())
    );
    let (st, h, _) = send(&app, req("GET", path, &boss, Some("bytes=99-"))).await;
    assert_eq!(st, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(h[header::CONTENT_RANGE], "bytes */40");
    // the same access rule as local files: a member who cannot see the meeting gets 404 and no bytes
    let (st, _, body) = send(&app, req("GET", path, &amy, None)).await;
    assert_eq!((st, body.len()), (StatusCode::NOT_FOUND, 0));

    let (st, ..) = send(&app, req("DELETE", "/api/meetings/m1", &boss, None)).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(
        objects.lock().unwrap().is_empty(),
        "the object is hard-deleted with the meeting"
    );
}

#[tokio::test]
async fn a_recording_over_one_chunk_goes_up_as_a_multipart_upload_and_comes_back_whole() {
    let dir = tempfile::tempdir().unwrap();
    let (url, objects) = mock_s3();
    let cfg = cfg(dir.path(), &url);
    let store = S3Store::from_config(&cfg.storage, tokio::runtime::Handle::current())
        .unwrap()
        .unwrap();
    // 20 MiB: more than the client's 8 MiB chunk, so it cannot be a single PUT
    let big: Vec<u8> = (0..20 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    let file = dir.path().join("big.wav");
    std::fs::write(&file, &big).unwrap();
    let f2 = file.clone();
    tokio::task::spawn_blocking(move || {
        use recapz_core::blobs::Blobs;
        store.put_file("t/audio/big/recording.wav", &f2).unwrap();
        store.delete("t/audio/never-existed.wav").unwrap(); // deleting a missing object is fine
    })
    .await
    .unwrap();
    let o = objects.lock().unwrap();
    let got = o
        .get("t/audio/big/recording.wav")
        .unwrap_or_else(|| panic!("object stored; have {:?}", o.keys().collect::<Vec<_>>()));
    assert_eq!(got.len(), big.len());
    assert!(got == &big, "parts were stitched in order");
}
