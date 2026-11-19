//! Local-only simulator for the live view: `POST /api/dev/live-sim` starts a fake call owned by the
//! caller and plays a script into the live transcript, `DELETE /api/dev/live-sim/{id}` ends it. No
//! browser, audio or speech provider is involved; everything after the live transcript (the live
//! view, Ask now, search, the language model) is the real code. Refused unless `dev_login` is on.
use super::*;
use crate::dev_call::{self, Stop as CallStop};
use crate::live::{hub, Msg};
use axum::extract::Query;
use std::time::{Duration, Instant};

/// Marks a meeting as simulated, so the stop route never touches a real one.
const MARKER: &str = "dev-simulator";
const MAX_LINES: usize = 200;
const MAX_LINE_CHARS: usize = 500;
/// An uploaded recording to replay: at most this many bytes and this many minutes of audio.
const MAX_AUDIO_BYTES: usize = 100 * 1024 * 1024;
const MAX_AUDIO_MINUTES: usize = 30;
pub const AUDIO_BODY_LIMIT: usize = MAX_AUDIO_BYTES + 64 * 1024;

/// Played when the request brings no script; the topics match plain sample documents about rates and terms.
const SAMPLE: [&str; 6] = [
    "Thanks for joining. Let's go through the quote for the new project.",
    "What is the hourly rate for the design work?",
    "And what are the payment terms if we start next month?",
    "Cho mình hỏi khoản đặt cọc phải trả trước buổi đầu tiên không?",
    "We also need to confirm the kickoff date.",
    "Can you send the onboarding notes before Monday?",
];

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Start {
    title: Option<String>,
    /// Lines to say, one every `interval_ms`; the built-in sample when empty.
    lines: Vec<String>,
    /// Pause between lines, 200 to 10000 ms.
    interval_ms: Option<u64>,
    /// Say the live transcript failed after this many lines (recording would go on).
    fail_after: Option<usize>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Stop {
    /// Act as a chat opt-out: the meeting is deleted instead of ended.
    optout: bool,
}

fn enabled(app: &App) -> Res<()> {
    if app.cfg.dev_login {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

pub async fn start(
    State(app): State<App>,
    jar: CookieJar,
    Json(b): Json<Start>,
) -> Res<(StatusCode, Json<Created>)> {
    enabled(&app)?;
    let u = app.current_user(&jar).await?;
    if b.lines.len() > MAX_LINES || b.lines.iter().any(|l| l.chars().count() > MAX_LINE_CHARS) {
        return Err(ApiError::BadRequest("at most 200 lines of 500 characters"));
    }
    let lines: Vec<String> = if b.lines.is_empty() {
        SAMPLE.map(String::from).to_vec()
    } else {
        b.lines
    };
    let every = Duration::from_millis(b.interval_ms.unwrap_or(2000).clamp(200, 10_000));
    let title = b
        .title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Simulated call".into());
    let id = db::new_id();
    let now = db::now_ms();
    let (mid, uid) = (id.clone(), u.id.clone());
    app.tenant(u.tenant_id.clone(), move |c| {
        ensure_member(c, &u)?;
        c.execute(
            "INSERT INTO meetings (id, title, platform, join_url, status, owner_id, started_at, created_at)
             VALUES (?1, ?2, 'meet', ?3, 'recording', ?4, ?5, ?5)",
            params![mid, title, MARKER, uid, now],
        )?;
        Ok(())
    })
    .await?;
    hub().start(&id);
    let (play, fail_after) = (id.clone(), b.fail_after);
    tokio::spawn(async move {
        let began = Instant::now();
        for (i, line) in lines.into_iter().enumerate() {
            if fail_after == Some(i) {
                hub().push(&play, Msg::Unavailable);
                return;
            }
            // Say the first half as it would arrive partially, then the whole line.
            let half: String = line.chars().take(line.chars().count() / 2).collect();
            hub().push(&play, Msg::Partial(half));
            tokio::time::sleep(every / 2).await;
            let at_ms = began.elapsed().as_millis() as i64;
            hub().push(&play, Msg::Final { at_ms, text: line });
            tokio::time::sleep(every / 2).await;
            // Stopped (or opted out) meanwhile: nothing left to play into.
            if hub().recent(&play, 0).is_none() {
                return;
            }
        }
    });
    Ok((StatusCode::CREATED, Json(Created { id })))
}

pub async fn stop(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
    Query(q): Query<Stop>,
) -> Res<StatusCode> {
    enabled(&app)?;
    let u = app.current_user(&jar).await?;
    let (dir, tenant) = (app.cfg.data_dir.clone(), u.tenant_id.clone());
    let store = app.store.clone();
    let mid = id.clone();
    let found = app
        .tenant(tenant.clone(), move |c| {
            ensure_member(c, &u)?;
            let owned: Option<String> = c
                .query_row(
                    "SELECT id FROM meetings WHERE id=?1 AND owner_id=?2 AND join_url=?3 AND status IN ('joining','recording')",
                    params![mid, u.id, MARKER],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(mid) = owned else { return Ok(false) };
            // A replayed recording ends itself the way a real call does, on its own thread.
            let how = if q.optout { CallStop::OptOut } else { CallStop::End };
            if dev_call::stop(&mid, how) {
                return Ok(true);
            }
            // The same order as a real call: the live transcript goes first, then the meeting.
            hub().clear(&mid);
            if q.optout {
                purge_meeting(
                    c,
                    &dir,
                    &tenant,
                    &mid,
                    store.as_deref().map(|s| s as &dyn recapz_core::blobs::Blobs),
                )?;
            } else {
                c.execute(
                    "UPDATE meetings SET status='ready', ended_at=?2 WHERE id=?1",
                    params![mid, db::now_ms()],
                )?;
            }
            Ok(true)
        })
        .await?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// Replays an uploaded recording as a fake call: `file` (any audio the upload screen accepts),
/// optional `title` and `language` (`vi`, `en` or `multi`). The audio goes to the speech provider
/// twice, as in a real call: streamed live, then in full after the call. Use only recordings the
/// people in them agreed to.
pub async fn start_audio(
    State(app): State<App>,
    jar: CookieJar,
    mut mp: Multipart,
) -> Res<(StatusCode, Json<Created>)> {
    enabled(&app)?;
    let u = app.current_user(&jar).await?;
    let (mut file, mut title, mut language) = (None, None, None);
    while let Some(f) = mp
        .next_field()
        .await
        .map_err(|_| ApiError::BadRequest("bad multipart body or file over 100 MB"))?
    {
        match f.name() {
            Some("file") => {
                let name = f.file_name().unwrap_or("call.wav").to_owned();
                let bytes = f
                    .bytes()
                    .await
                    .map_err(|_| ApiError::BadRequest("bad multipart body or file over 100 MB"))?;
                file = Some((name, bytes));
            }
            Some("title") => title = f.text().await.ok(),
            Some("language") => language = f.text().await.ok(),
            _ => {}
        }
    }
    let (name, bytes) = file.ok_or(ApiError::BadRequest("missing file field"))?;
    let language = meeting_language(language.as_deref())?;
    if app.cfg.speech.api_key.trim().is_empty() {
        return Err(ApiError::Unavailable(
            "the simulated call needs RECAPZ_SPEECH_KEY",
        ));
    }
    // The decoder picks the format from the extension, so keep a short, plain one.
    let ext: String = name
        .rsplit('.')
        .next()
        .filter(|e| (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("wav")
        .to_lowercase();
    let tmp_dir = app.cfg.data_dir.join("notetaker-tmp");
    let tmp = tmp_dir.join(format!("dev-{}.{ext}", db::new_id()));
    let samples = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&tmp_dir)?;
        std::fs::write(&tmp, &bytes)?;
        let decoded = recapz_speech::decode::decode_to_16k_mono(&tmp);
        let _ = std::fs::remove_file(&tmp);
        decoded
    })
    .await?
    .map_err(|_| ApiError::BadRequest("could not read that audio file"))?;
    if samples.is_empty() || samples.len() > MAX_AUDIO_MINUTES * 60 * 16_000 {
        return Err(ApiError::BadRequest(
            "the audio must be 1 second to 30 minutes",
        ));
    }
    let title = title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Simulated recording".into());
    let id = db::new_id();
    let (mid, uid, tenant) = (id.clone(), u.id.clone(), u.tenant_id.clone());
    app.tenant(tenant.clone(), move |c| {
        ensure_member(c, &u)?;
        c.execute(
            "INSERT INTO meetings (id, title, platform, join_url, status, owner_id, language, created_at)
             VALUES (?1, ?2, 'meet', ?3, 'joining', ?4, ?5, ?6)",
            params![mid, title, MARKER, uid, language, db::now_ms()],
        )?;
        Ok(())
    })
    .await?;
    let live = crate::notetaker::Live {
        base_url: app.cfg.speech.listen_url(),
        api_key: app.cfg.speech.api_key.clone(),
    };
    let max_call_ms = i64::from(app.cfg.max_call_minutes.max(1)) * 60_000;
    dev_call::spawn(
        app.cfg.data_dir.clone(),
        tenant,
        id.clone(),
        samples,
        live,
        max_call_ms,
    )
    .map_err(|e| ApiError::Internal(e.into()))?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}
