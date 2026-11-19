//! Browser recording: `POST /api/recordings` starts a call owned by the caller, and
//! `GET /api/meetings/{id}/record` is the WebSocket its page streams audio over. Frames go to the
//! call's `BrowserDriver`; the call itself (recording, live transcript, jobs) is the shared pipeline.
use super::*;
use crate::browser_call::{self, Stop};
use axum::extract::ws::{close_code, CloseFrame, Message, WebSocket, WebSocketUpgrade};
use std::{collections::HashSet, sync::OnceLock, time::Duration};

/// One frame is 100 ms of 16 kHz mono audio: 1600 little-endian i16 samples.
const FRAME_BYTES: usize = 1600 * 2;
/// A socket that sends nothing for this long is dropped, so a reconnect is not refused as a second
/// socket while a dead one lingers. The page sends ten frames a second.
const IDLE: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
pub struct Start {
    title: String,
    language: Option<String>,
    /// `tab` (tab audio and microphone) or `room` (microphone only). Both stream the same mono frames,
    /// so the server only checks it.
    mode: String,
}

pub async fn start(
    State(app): State<App>,
    jar: CookieJar,
    Json(b): Json<Start>,
) -> Res<(StatusCode, Json<Created>)> {
    let u = app.current_user(&jar).await?;
    let title = b.title.trim().to_string();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return Err(ApiError::BadRequest("title must be 1 to 200 characters"));
    }
    if !matches!(b.mode.as_str(), "tab" | "room") {
        return Err(ApiError::BadRequest("mode must be tab or room"));
    }
    let language = meeting_language(b.language.as_deref())?;
    let id = db::new_id();
    let (mid, tenant) = (id.clone(), u.tenant_id.clone());
    app.tenant(tenant.clone(), move |c| {
        ensure_member(c, &u)?;
        // Recording from the start: the page connects as soon as this returns, and the driver is
        // admitted at once.
        c.execute(
            "INSERT INTO meetings (id, title, platform, status, owner_id, language, started_at, created_at)
             VALUES (?1, ?2, 'browser', 'recording', ?3, ?4, ?5, ?5)",
            params![mid, title, u.id, language, db::now_ms()],
        )?;
        Ok(())
    })
    .await?;
    // ponytail: no speech key means no live transcript; the call still records and is transcribed later.
    let live = (!app.cfg.speech.api_key.trim().is_empty()).then(|| crate::notetaker::Live {
        base_url: app.cfg.speech.listen_url(),
        api_key: app.cfg.speech.api_key.clone(),
    });
    let max_call_ms = i64::from(app.cfg.max_call_minutes.max(1)) * 60_000;
    if let Err(e) = browser_call::spawn(
        app.cfg.data_dir.clone(),
        tenant.clone(),
        id.clone(),
        live,
        max_call_ms,
    ) {
        let (dir, mid) = (app.cfg.data_dir.clone(), id.clone());
        app.tenant(tenant.clone(), move |c| {
            crate::notetaker::fail(c, &dir, &tenant, &mid, "The recording could not start")
        })
        .await?;
        return Err(ApiError::Internal(e.into()));
    }
    tracing::info!(meeting = %id, "browser recording started");
    Ok((StatusCode::CREATED, Json(Created { id })))
}

type Sockets = std::sync::Mutex<HashSet<String>>;

fn sockets() -> &'static Sockets {
    static S: OnceLock<Sockets> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// The one open socket of a meeting; freed when dropped.
struct Slot(String);

impl Slot {
    fn take(id: &str) -> Option<Self> {
        sockets()
            .lock()
            .unwrap()
            .insert(id.to_string())
            .then(|| Slot(id.to_string()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        sockets().lock().unwrap().remove(&self.0);
    }
}

pub async fn socket(
    State(app): State<App>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Res<Response> {
    // Cookies ride along on a cross-site WebSocket, so the origin is the only CSRF check here.
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if origin != Some(app.cfg.base_url.trim_end_matches('/')) {
        return Err(ApiError::Denied("bad origin"));
    }
    let u = app.current_user(&jar).await?;
    let (uid, mid) = (u.id.clone(), id.clone());
    let status = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            // Owner only: a workspace share lets others watch, never record.
            Ok(c.query_row(
                "SELECT platform = 'browser' AND status = 'recording' FROM meetings
                 WHERE id = ?1 AND owner_id = ?2",
                params![mid, uid],
                |r| r.get::<_, bool>(0),
            )
            .optional()?)
        })
        .await?
        .ok_or(ApiError::NotFound)?;
    if !status {
        return Err(ApiError::Conflict("meeting is not recording"));
    }
    let slot = Slot::take(&id).ok_or(ApiError::Conflict("already recording from another page"))?;
    let frames = browser_call::attach(&id).ok_or(ApiError::Conflict("meeting is not recording"))?;
    Ok(ws.on_upgrade(move |s| pump(s, slot, frames)))
}

async fn pump(mut ws: WebSocket, slot: Slot, frames: tokio::sync::mpsc::UnboundedSender<Vec<i16>>) {
    let id = &slot.0;
    let code = loop {
        let msg = match tokio::time::timeout(IDLE, ws.recv()).await {
            Ok(Some(Ok(m))) => m,
            _ => break close_code::AWAY, // closed, broken or idle: the call waits for a reconnect
        };
        match msg {
            Message::Binary(b) if b.len() == FRAME_BYTES => {
                let samples = b
                    .as_chunks()
                    .0
                    .iter()
                    .map(|p| i16::from_le_bytes(*p))
                    .collect();
                if frames.send(samples).is_err() {
                    break close_code::NORMAL; // the call ended
                }
            }
            Message::Text(t) if t.as_str() == "stop" || t.as_str() == "discard" => {
                let how = if t.as_str() == "stop" {
                    Stop::End
                } else {
                    Stop::Discard
                };
                browser_call::stop(id, how);
                break close_code::NORMAL;
            }
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => break close_code::NORMAL,
            _ => {
                tracing::warn!(meeting = %id, "recording socket sent a bad message");
                break close_code::POLICY;
            }
        }
    };
    let _ = ws
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: "".into(),
        })))
        .await;
}
