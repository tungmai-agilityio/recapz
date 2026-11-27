//! `POST /api/ask`: answer a question from the transcripts the member may see and their own documents.
//! The reply is a server-sent event stream: `token` events with the answer text, one `citations`
//! event with the moments used, then `done`. Retrieval applies the access rule inside the query.
use super::*;
use recapz_core::{ask_history, search};
use recapz_llm::ask::{answer_stream, Excerpt, Source};
use tokio::io::AsyncWriteExt;

const TOP_K: usize = 6;
const QUOTE_CHARS: usize = 240;
const MAX_QUESTION_CHARS: usize = 1000;

#[derive(Deserialize)]
pub struct AskBody {
    question: String,
    /// Optional narrowing: UTC dates `YYYY-MM-DD` (inclusive) and a speaker's name.
    from: Option<String>,
    to: Option<String>,
    speaker: Option<String>,
}

/// An empty value means "no filter"; a date must really exist (`2026-02-30` is refused).
fn date(v: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(v) = v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    // The format check keeps `2026-1-5` and a trailing time out, which `Date::parse` would also refuse.
    let ok = time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
        .ok()
        .is_some_and(|f| time::Date::parse(&v, &f).is_ok());
    if ok {
        Ok(Some(v))
    } else {
        Err(ApiError::BadRequest("dates must look like 2026-11-30"))
    }
}

fn filter(b: &mut AskBody) -> Result<search::Filter, ApiError> {
    let speaker = b
        .speaker
        .take()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    if speaker.as_ref().is_some_and(|s| s.chars().count() > 100) {
        return Err(ApiError::BadRequest(
            "person must be at most 100 characters",
        ));
    }
    Ok(search::Filter {
        from: date(b.from.take())?,
        to: date(b.to.take())?,
        speaker,
    })
}

pub(super) fn event(name: &str, data: serde_json::Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

/// One `llm_usage` row per model call; a failure to record is logged, never shown to the member.
pub(super) async fn record_usage(
    app: &App,
    tenant: String,
    task: &'static str,
    calls: Vec<recapz_llm::Routed>,
) {
    if let Err(ApiError::Internal(e)) = app
        .tenant(tenant, move |c| {
            for k in &calls {
                c.execute(
                    "INSERT INTO llm_usage (task, provider, model, input_tokens, output_tokens, cost_micro_usd, at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        task,
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
        tracing::error!("{task} usage not recorded: {e:#}");
    }
}

/// The citation card for one hit; `answered` is false for a passage shown without a generated answer.
pub(super) fn citation(h: &search::Hit, answered: bool) -> serde_json::Value {
    serde_json::json!({
        "meeting_id": h.meeting_id,
        "document_id": h.document_id,
        "title": h.title,
        "start_ms": h.meeting_id.as_ref().map(|_| h.start_ms),
        "quote": h.text.chars().take(QUOTE_CHARS).collect::<String>(),
        "answered": answered,
    })
}

const RIBBON_PARTS: i64 = 60;

/// Who spoke when, in at most `RIBBON_PARTS` equal slices of `[0, duration_ms]`: each slice goes to
/// the speaker with the most time in it (`None` when nobody spoke) and neighbours with the same
/// speaker merge, so `weight` is the number of slices. Speakers are numbered from 0 by first
/// appearance, the numbering `web/src/ui/Ribbon.tsx` colours by.
fn ribbon(segs: &[(Option<String>, i64, i64)], duration_ms: i64) -> Vec<(Option<usize>, i64)> {
    let mut names: Vec<&str> = vec![];
    let mut spans: Vec<(Option<usize>, i64, i64)> = vec![];
    for (who, a, b) in segs {
        let i = who
            .as_deref()
            .map(|w| match names.iter().position(|n| *n == w) {
                Some(i) => i,
                None => {
                    names.push(w);
                    names.len() - 1
                }
            });
        spans.push((i, *a, *b));
    }
    let mut out: Vec<(Option<usize>, i64)> = vec![];
    for k in 0..RIBBON_PARTS {
        let (lo, hi) = (
            duration_ms * k / RIBBON_PARTS,
            duration_ms * (k + 1) / RIBBON_PARTS,
        );
        let mut time = vec![0i64; names.len()];
        for (i, a, b) in &spans {
            if let Some(i) = i {
                time[*i] += (b.min(&hi) - a.max(&lo)).max(0);
            }
        }
        // First speaker wins a tie.
        let best = time
            .iter()
            .enumerate()
            .rev()
            .max_by_key(|(_, t)| **t)
            .filter(|(_, t)| **t > 0);
        let who = best.map(|(i, _)| i);
        match out.last_mut() {
            Some((w, n)) if *w == who => *n += 1,
            _ => out.push((who, 1)),
        }
    }
    out
}

/// A citation card with the data for the web app: its marker number `n` (the excerpt number the
/// model used in the text), and for a meeting the speaker at that moment, the length of the
/// transcript and the ribbon. A document, or a meeting without a transcript, leaves them null.
fn card(
    c: &Connection,
    h: &search::Hit,
    n: Option<usize>,
    answered: bool,
) -> anyhow::Result<serde_json::Value> {
    let mut v = citation(h, answered);
    v["n"] = serde_json::json!(n);
    for k in ["speaker", "duration_ms", "ribbon"] {
        v[k] = serde_json::Value::Null;
    }
    let Some(id) = &h.meeting_id else {
        return Ok(v);
    };
    let mut q = c.prepare(
        "SELECT COALESCE(p.display_name, p.label), g.start_ms, g.end_ms
         FROM segments g LEFT JOIN speakers p ON p.id = g.speaker_id
         WHERE g.meeting_id = ?1 ORDER BY g.start_ms, g.id",
    )?;
    let segs = q
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<(Option<String>, i64, i64)>, _>>()?;
    let duration = segs.iter().map(|s| s.2).max().unwrap_or(0);
    if duration <= 0 {
        return Ok(v);
    }
    let overlap = |s: &&(Option<String>, i64, i64)| s.2.min(h.end_ms) - s.1.max(h.start_ms);
    v["speaker"] = serde_json::json!(segs
        .iter()
        .filter(|s| s.0.is_some() && overlap(s) > 0)
        .max_by_key(|s| (overlap(s), std::cmp::Reverse(s.1)))
        .and_then(|s| s.0.clone()));
    v["duration_ms"] = serde_json::json!(duration);
    v["ribbon"] = ribbon(&segs, duration)
        .into_iter()
        .map(|(speaker, weight)| serde_json::json!({ "speaker": speaker, "weight": weight }))
        .collect();
    Ok(v)
}

/// The excerpts a model may see: only hits whose owner allowed it (`sendable`), the rest are
/// returned as passages to show without a generated answer.
pub(super) fn split_hits(hits: Vec<search::Hit>) -> (Vec<search::Hit>, Vec<search::Hit>) {
    hits.into_iter().partition(|h| h.llm_ok)
}

pub(super) fn excerpts(hits: &[search::Hit]) -> Vec<Excerpt> {
    hits.iter()
        .map(|h| Excerpt {
            source: if h.meeting_id.is_some() {
                Source::Meeting {
                    start_ms: h.start_ms,
                }
            } else {
                Source::Document {
                    title: h.title.clone(),
                }
            },
            text: h.text.clone(),
        })
        .collect()
}

pub async fn ask(
    State(app): State<App>,
    jar: CookieJar,
    Json(mut body): Json<AskBody>,
) -> Res<Response> {
    let u = app.current_user(&jar).await?;
    let question = body.question.trim().to_string();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        return Err(ApiError::BadRequest(
            "question must be 1 to 1000 characters",
        ));
    }
    let filter = filter(&mut body)?;
    // After validation so a malformed request does not burn quota; before any retrieval or model call.
    if !app.allow_ask(&u.id) {
        return Err(ApiError::TooManyRequests);
    }
    let router = app.llm.clone().ok_or(ApiError::Unavailable(
        "Ask needs a language model; set [llm] in config",
    ))?;

    let member = u.id.clone();
    let (uid, q, tenant) = (u.id.clone(), question.clone(), u.tenant_id.clone());
    let embedder = app.embedder.clone();
    let hits = app
        .tenant(tenant.clone(), move |c| {
            ensure_member(c, &u)?;
            search::search_with(c, &uid, &q, embedder.as_ref(), TOP_K, &filter)
        })
        .await?;
    // Only text the owner allowed to leave the server is sent to the model; a toggle-off document
    // is shown as a cited passage after the answer. `cited` indexes into `sendable`.
    let (sendable, passages) = split_hits(hits);
    let excerpts = excerpts(&sendable);

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
                record_usage(&app, tenant.clone(), "ask_answer", out.calls).await;
                // `n` is the number the model saw for the excerpt: its place in `sendable` plus one.
                let cards: Vec<_> = out
                    .value
                    .cited
                    .iter()
                    .map(|&i| (sendable[i].clone(), Some(i + 1), true))
                    .chain(passages.iter().map(|h| (h.clone(), None, false)))
                    .collect();
                let plain: Vec<_> = cards.iter().map(|(h, _, a)| citation(h, *a)).collect();
                let citations = app
                    .tenant(tenant.clone(), move |c| {
                        cards
                            .iter()
                            .map(|(h, n, a)| card(c, h, *n, *a))
                            .collect::<anyhow::Result<Vec<_>>>()
                    })
                    .await
                    .unwrap_or_else(|_| {
                        tracing::error!("citation details not loaded");
                        plain
                    });
                // Saved here, not by the client, so a closed tab still keeps the turn. Only a
                // completed answer is saved; the `done` event carries the turn's id.
                let citations = serde_json::Value::Array(citations);
                let (who, cards_json, answer) =
                    (member, citations.to_string(), out.value.answer.clone());
                let id = app
                    .tenant(tenant.clone(), move |c| {
                        ask_history::save(c, &who, &question, &answer, &cards_json)
                    })
                    .await
                    .map_err(|_| tracing::error!("ask turn not saved"))
                    .ok();
                event("citations", citations) + &event("done", serde_json::json!({ "id": id }))
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

#[derive(Serialize)]
pub struct HistoryTurn {
    id: String,
    question: String,
    answer: String,
    citations: serde_json::Value,
    created_at: i64,
}

/// The member's saved turns, oldest first. A turn that cites a meeting the member can no longer see
/// is left out (not deleted: it returns if access returns).
pub async fn history(State(app): State<App>, jar: CookieJar) -> Res<Json<Vec<HistoryTurn>>> {
    let u = app.current_user(&jar).await?;
    let rows = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            let mut visible = c.prepare(&format!(
                "SELECT 1 FROM meetings m WHERE m.id = ?2 AND {}",
                db::MEETING_VISIBLE
            ))?;
            let mut out = vec![];
            for t in ask_history::list(c, &u.id)? {
                let cites: serde_json::Value = serde_json::from_str(&t.citations_json)?;
                let hidden = {
                    let mut hidden = false;
                    for m in cites
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v["meeting_id"].as_str())
                    {
                        if !visible.exists([&u.id, m])? {
                            hidden = true;
                            break;
                        }
                    }
                    hidden
                };
                if !hidden {
                    out.push(HistoryTurn {
                        id: t.id,
                        question: t.question,
                        answer: t.answer,
                        citations: cites,
                        created_at: t.created_at,
                    });
                }
            }
            Ok(out)
        })
        .await?;
    Ok(Json(rows))
}

/// Deletes one of the member's own turns; 404 for another member's or a missing one.
pub async fn delete_turn(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<StatusCode> {
    let u = app.current_user(&jar).await?;
    let found = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            ask_history::delete_one(c, &u.id, &id)
        })
        .await?;
    found
        .then_some(StatusCode::NO_CONTENT)
        .ok_or(ApiError::NotFound)
}

pub async fn clear_history(State(app): State<App>, jar: CookieJar) -> Res<StatusCode> {
    let u = app.current_user(&jar).await?;
    app.tenant(u.tenant_id.clone(), move |c| {
        ensure_member(c, &u)?;
        ask_history::clear(c, &u.id)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(who: Option<&str>, a: i64, b: i64) -> (Option<String>, i64, i64) {
        (who.map(Into::into), a, b)
    }

    #[test]
    fn ribbon_merges_neighbours_marks_silence_and_numbers_speakers_by_first_appearance() {
        // 0-60 s: Bo, 60-120 s: nobody, 120-180 s: Ann then Bo again at the very end.
        let r = ribbon(
            &[
                seg(Some("Bo"), 0, 60_000),
                seg(Some("Ann"), 120_000, 170_000),
                seg(Some("Bo"), 170_000, 180_000),
            ],
            180_000,
        );
        assert_eq!(
            r,
            vec![(Some(0), 20), (None, 20), (Some(1), 17), (Some(0), 3)]
        );
        assert_eq!(r.iter().map(|p| p.1).sum::<i64>(), RIBBON_PARTS);
    }

    #[test]
    fn ribbon_of_an_untitled_speakerless_transcript_is_one_unknown_part() {
        assert_eq!(ribbon(&[seg(None, 0, 1000)], 1000), vec![(None, 60)]);
    }
}
