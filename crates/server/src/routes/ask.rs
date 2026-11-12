//! `POST /api/ask`: answer a question from the transcripts the member may see.
//! The reply is a server-sent event stream: `token` events with the answer text, one `citations`
//! event with the moments used, then `done`. Retrieval applies the access rule inside the query.
use super::*;
use recapz_core::search;
use recapz_llm::ask::{answer_stream, Excerpt};
use tokio::io::AsyncWriteExt;

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
    // After validation so a malformed request does not burn quota; before any retrieval or model call.
    if !app.allow_ask(&u.id) {
        return Err(ApiError::TooManyRequests);
    }
    let router = app.llm.clone().ok_or(ApiError::Unavailable(
        "Ask needs a language model; set [llm] in config",
    ))?;

    let (uid, q, tenant) = (u.id.clone(), question.clone(), u.tenant_id.clone());
    let embedder = app.embedder.clone();
    let hits = app
        .tenant(tenant.clone(), move |c| {
            ensure_member(c, &u)?;
            search::search(c, &uid, &q, embedder.as_ref(), TOP_K)
        })
        .await?;
    let excerpts: Vec<Excerpt> = hits
        .iter()
        .map(|h| Excerpt {
            start_ms: h.start_ms,
            text: h.text.clone(),
        })
        .collect();

    // Everything that can fail with a proper status is done; from here the reply is a stream.
    // A task writes events into a pipe the response reads from. If the reader goes away the model
    // call still finishes, so its cost is recorded.
    let (mut tx_pipe, rx_pipe) = tokio::io::duplex(8 * 1024);
    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let produce = async {
            answer_stream(&router, &question, &excerpts, &mut move |t| {
                let _ = tx.send(event("token", serde_json::json!({ "text": t })));
            })
            .await
        };
        let consume = async {
            while let Some(e) = rx.recv().await {
                if tx_pipe.write_all(e.as_bytes()).await.is_err() {
                    break;
                }
            }
        };
        let (res, ()) = tokio::join!(produce, consume);
        let tail = match res {
            Ok(out) => {
                let calls = out.calls;
                if let Err(ApiError::Internal(e)) = app
                    .tenant(tenant, move |c| {
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
                    .await
                {
                    tracing::error!("ask usage not recorded: {e:#}");
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
                event("citations", serde_json::Value::Array(citations))
                    + &event("done", serde_json::json!({}))
            }
            Err(e) => {
                tracing::error!("ask failed: {e}");
                event(
                    "error",
                    serde_json::json!({ "message": "the answer could not be completed" }),
                )
            }
        };
        let _ = tx_pipe.write_all(tail.as_bytes()).await;
    });
    Ok((
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
            // Tells a reverse proxy not to hold the stream back.
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Body::from_stream(ReaderStream::new(rx_pipe)),
    )
        .into_response())
}
