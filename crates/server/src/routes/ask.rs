//! `POST /api/ask`: answer a question from the transcripts the member may see.
//! The reply is a server-sent event stream: `token` events with the answer text, one `citations`
//! event with the moments used, then `done`. Retrieval applies the access rule inside the query.
use super::*;
use recapz_core::{
    embed::{Embedder, HashEmbedder},
    search,
};
use recapz_llm::ask::{answer, Excerpt};

const TOP_K: usize = 6;
const QUOTE_CHARS: usize = 240;
const MAX_QUESTION_CHARS: usize = 1000;

#[derive(Deserialize)]
pub struct AskBody {
    question: String,
}

fn event(name: &str, data: serde_json::Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

pub async fn ask(
    State(app): State<App>,
    jar: CookieJar,
    Json(body): Json<AskBody>,
) -> Res<Response> {
    let u = app.current_user(&jar).await?;
    let question = body.question.trim().to_string();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        return Err(ApiError::BadRequest(
            "question must be 1 to 1000 characters",
        ));
    }
    let router = app.llm.clone().ok_or(ApiError::Unavailable(
        "Ask needs a language model; set [llm] in config",
    ))?;

    let (uid, q, tenant) = (u.id.clone(), question.clone(), u.tenant_id.clone());
    let hits = app
        .tenant(tenant.clone(), move |c| {
            ensure_member(c, &u)?;
            search::search(c, &uid, &q, &HashEmbedder.embed(&q), TOP_K)
        })
        .await?;
    let excerpts: Vec<Excerpt> = hits
        .iter()
        .map(|h| Excerpt {
            title: h.title.clone(),
            start_ms: h.start_ms,
            text: h.text.clone(),
        })
        .collect();
    let out = answer(&router, &question, &excerpts)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let calls = out.calls;
    app.tenant(tenant, move |c| {
        for k in &calls {
            c.execute(
                "INSERT INTO llm_usage (task, provider, model, input_tokens, output_tokens, cost_micro_usd, at)
                 VALUES ('ask_answer', ?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    k.provider,
                    k.model,
                    k.completion.input_tokens as i64,
                    k.completion.output_tokens as i64,
                    k.cost_micro_usd as i64,
                    db::now_ms()
                ],
            )?;
        }
        Ok(())
    })
    .await?;

    let mut sse = String::new();
    // ponytail: the model reply is complete before it is sent, so tokens are word-sized slices of it;
    // switch to provider streaming when answer latency matters.
    for word in out.value.answer.split_inclusive(' ') {
        sse.push_str(&event("token", serde_json::json!({ "text": word })));
    }
    let citations: Vec<_> = out
        .value
        .cited
        .iter()
        .map(|&i| {
            let h = &hits[i];
            serde_json::json!({
                "meeting_id": h.meeting_id,
                "title": h.title,
                "start_ms": h.start_ms,
                "quote": h.text.chars().take(QUOTE_CHARS).collect::<String>(),
            })
        })
        .collect();
    sse.push_str(&event("citations", serde_json::Value::Array(citations)));
    sse.push_str(&event("done", serde_json::json!({})));
    Ok((
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        sse,
    )
        .into_response())
}
