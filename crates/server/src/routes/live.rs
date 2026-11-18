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
        Msg::Final { at_ms, text } => {
            event("final", serde_json::json!({ "at_ms": at_ms, "text": text }))
        }
        Msg::Unavailable => event("unavailable", serde_json::json!({})),
        Msg::End => event("end", serde_json::json!({})),
    }
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
            .map(|(at_ms, text)| Msg::Final { at_ms, text })
            .collect();
        if !snap.partial.is_empty() {
            head.push(Msg::Partial(snap.partial));
        }
        if snap.unavailable {
            head.push(Msg::Unavailable);
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
