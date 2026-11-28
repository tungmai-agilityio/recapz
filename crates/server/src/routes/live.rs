//! `GET /api/meetings/{id}/live`: the live transcript of a call in progress as server-sent events.
//! `final` (`at_ms`, `text`) and `partial` (`text`) carry text, `unavailable` says streaming failed
//! while recording goes on, and `end` closes the stream. A viewer first gets what was said so far.
use super::*;
use crate::live::{hub, Msg};
use tokio::{io::AsyncWriteExt, sync::broadcast::error::RecvError};

fn event(name: &str, data: serde_json::Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

fn render(m: &Msg) -> String {
    match m {
        Msg::Partial(t) => event("partial", serde_json::json!({ "text": t })),
        Msg::Final { at_ms, text, who } => event(
            "final",
            match who {
                Some(w) => {
                    serde_json::json!({ "at_ms": at_ms, "text": text, "speaker": w.n, "label": w.label, "ms": w.ms })
                }
                None => serde_json::json!({ "at_ms": at_ms, "text": text }),
            },
        ),
        Msg::Unavailable => event("unavailable", serde_json::json!({})),
        Msg::Paused { at_ms } => event("paused", serde_json::json!({ "at_ms": at_ms })),
        Msg::Resumed { at_ms } => event("resumed", serde_json::json!({ "at_ms": at_ms })),
        Msg::End => event("end", serde_json::json!({})),
    }
}

/// What the owner or an admin can do to a call in progress.
#[derive(Clone, Copy)]
pub enum Control {
    Pause,
    Resume,
    Stop,
}

/// `POST /api/meetings/{id}/live/{pause,resume,stop}`: owner or admin only (a meeting the member may
/// not see is a 404, one they see but do not own a 403). 404 as well when the call is not live. Pause
/// and resume repeat harmlessly; stop makes the notetaker leave and the meeting goes on to transcription.
pub async fn control(app: App, jar: CookieJar, id: String, what: Control) -> Res<StatusCode> {
    let u = app.current_user(&jar).await?;
    let (uid, role, mid) = (u.id.clone(), u.role.clone(), id.clone());
    let allowed = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            if visible_meeting(c, &uid, &mid)?.is_none() {
                return Ok(None);
            }
            let owner: Option<String> =
                c.query_row("SELECT owner_id FROM meetings WHERE id = ?1", [&mid], |r| {
                    r.get(0)
                })?;
            Ok(Some(
                owner.as_deref() == Some(uid.as_str()) || role == "admin",
            ))
        })
        .await?;
    match allowed {
        None => return Err(ApiError::NotFound),
        Some(false) => {
            return Err(ApiError::Denied(
                "only the owner or an admin can control this recording",
            ))
        }
        Some(true) => {}
    }
    let done = match what {
        Control::Pause => hub().set_paused(&id, true),
        Control::Resume => hub().set_paused(&id, false),
        Control::Stop => hub().request_stop(&id),
    };
    done.ok_or(ApiError::NotFound)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pause(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<StatusCode> {
    control(app, jar, id, Control::Pause).await
}

pub async fn resume(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<StatusCode> {
    control(app, jar, id, Control::Resume).await
}

pub async fn stop(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<StatusCode> {
    control(app, jar, id, Control::Stop).await
}

pub async fn live(State(app): State<App>, jar: CookieJar, Path(id): Path<String>) -> Res<Response> {
    let u = app.current_user(&jar).await?;
    let (uid, mid) = (u.id.clone(), id.clone());
    // The same access rule as the meeting page; a meeting the member may not see is reported as missing.
    let visible = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            Ok(visible_meeting(c, &uid, &mid)?.is_some())
        })
        .await?;
    if !visible {
        return Err(ApiError::NotFound);
    }
    let snap = hub().subscribe(&id).ok_or(ApiError::NotFound)?;
    let mut rx = snap.rx;
    let (mut pipe, reader) = tokio::io::duplex(8 * 1024);
    tokio::spawn(async move {
        let mut head: Vec<Msg> = snap
            .lines
            .into_iter()
            .map(|(at_ms, text, who)| Msg::Final { at_ms, text, who })
            .collect();
        if !snap.partial.is_empty() {
            head.push(Msg::Partial(snap.partial));
        }
        if snap.unavailable {
            head.push(Msg::Unavailable);
        }
        for (from, to) in snap.gaps {
            head.push(Msg::Paused { at_ms: from });
            if let Some(at_ms) = to {
                head.push(Msg::Resumed { at_ms });
            }
        }
        for m in head {
            if pipe.write_all(render(&m).as_bytes()).await.is_err() {
                return;
            }
        }
        loop {
            let m = match rx.recv().await {
                Ok(m) => m,
                // A slow viewer misses some lines; the stored transcript is the record.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => Msg::End,
            };
            let end = m == Msg::End;
            if pipe.write_all(render(&m).as_bytes()).await.is_err() || end {
                return;
            }
        }
    });
    Ok((
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Body::from_stream(ReaderStream::new(reader)),
    )
        .into_response())
}
