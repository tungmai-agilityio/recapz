//! `POST /api/meetings/{id}/live-ask`: a short cited answer to what was just said in a live call, from
//! the member's own documents and the meetings they can see. The reply is the same event stream as
//! Ask (`token`, `citations`, `done`) and is private to the member who asked: nothing about it is stored
//! except the usage row, and the live transcript is only read, never changed.
use super::ask::{citation, event, excerpts, record_usage, split_hits};
use super::*;
use crate::live::hub;
use recapz_core::search;
use recapz_llm::ask::live_answer;

const TOP_K: usize = 6;
/// "The last 60 s of live transcript".
const WINDOW_MS: i64 = 60_000;
/// Lines either side of a clicked line.
const NEIGHBOURS: usize = 2;
const MAX_TYPED_CHARS: usize = 1000;
const MAX_QUESTION_CHARS: usize = 4000;

#[derive(Deserialize)]
pub struct LiveAskBody {
    /// Optional typed question, added to the transcript.
    text: Option<String>,
    /// A clicked transcript line (its `at_ms`): use it and its neighbours instead of the last 60 s.
    at_ms: Option<i64>,
}

pub async fn live_ask(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(body): Json<LiveAskBody>,
) -> Res<Response> {
    let u = app.current_user(&jar).await?;
    let (uid, mid, tenant) = (u.id.clone(), id.clone(), u.tenant_id.clone());
    // The same access rule as the meeting page; a meeting the member may not see is reported as missing.
    let (visible, u) = app
        .tenant(tenant.clone(), move |c| {
            ensure_member(c, &u)?;
            Ok((visible_meeting(c, &uid, &mid)?.is_some(), u))
        })
        .await?;
    let said = match body.at_ms {
        Some(at) => hub().around(&id, at, NEIGHBOURS),
        None => hub().recent(&id, WINDOW_MS),
    };
    let (true, Some(said)) = (visible, said) else {
        return Err(ApiError::NotFound);
    };
    let typed = body.text.unwrap_or_default().trim().to_string();
    if typed.chars().count() > MAX_TYPED_CHARS {
        return Err(ApiError::BadRequest("text must be at most 1000 characters"));
    }
    let question: String = said
        .into_iter()
        .chain(Some(typed).filter(|t| !t.is_empty()))
        .collect::<Vec<_>>()
        .join(" ");
    if question.is_empty() {
        return Err(ApiError::BadRequest("nothing has been said yet"));
    }
    // Keep the end: the most recent words matter most.
    let skip = question.chars().count().saturating_sub(MAX_QUESTION_CHARS);
    let question: String = question.chars().skip(skip).collect();
    // The Ask quota: after validation so a malformed request does not burn it, before any retrieval or model call.
    if !app.allow_ask(&u.id) {
        return Err(ApiError::TooManyRequests);
    }
    let router = app.llm.clone().ok_or(ApiError::Unavailable(
        "Ask needs a language model; set [llm] in config",
    ))?;

    let (uid, q, embedder) = (u.id.clone(), question.clone(), app.embedder.clone());
    let hits = app
        .tenant(tenant.clone(), move |c| {
            ensure_member(c, &u)?;
            search::search(c, &uid, &q, embedder.as_ref(), TOP_K)
        })
        .await?;
    // Only text the owner allowed to leave the server reaches the model; the rest is shown as cited passages.
    let (sendable, passages) = split_hits(hits);
    let tail = match live_answer(&router, &question, &excerpts(&sendable)).await {
        Ok(out) => {
            record_usage(&app, tenant, "live_answer", out.calls).await;
            let cards: Vec<_> = out
                .value
                .cited
                .iter()
                .map(|&i| citation(&sendable[i], true))
                .chain(passages.iter().map(|h| citation(h, false)))
                .collect();
            event("token", serde_json::json!({ "text": out.value.answer }))
                + &event("citations", serde_json::Value::Array(cards))
                + &event("done", serde_json::json!({}))
        }
        Err(e) => {
            tracing::error!("live answer failed: {e}");
            event(
                "error",
                serde_json::json!({ "message": "the answer could not be completed" }),
            )
        }
    };
    // ponytail: one short answer, so the events go out together; stream tokens if the 6 s bar needs it.
    Ok((
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        tail,
    )
        .into_response())
}
