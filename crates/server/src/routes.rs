use crate::config::Config;
mod calendar;
mod desktop_auth;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, patch, post},
    Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use recapz_core::{db, jobs, purge::purge_meeting};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

const SESSION_COOKIE: &str = "recapz_session";
const CSRF_COOKIE: &str = "recapz_csrf";
const CSRF_HEADER: &str = "x-csrf-token";
const OAUTH_STATE_COOKIE: &str = "recapz_oauth_state";
const SESSION_TTL_MS: i64 = 30 * 24 * 3600 * 1000;
const MAX_UPLOAD_BYTES: usize = 2 * 1024 * 1024 * 1024;

#[derive(Clone)]
pub struct App {
    cfg: Arc<Config>,
    // ponytail: one shared control connection behind a mutex; fine for one team, pool if it ever contends.
    control: Arc<Mutex<Connection>>,
    /// Language-model router for Ask; `None` when no provider is configured.
    llm: Option<Arc<recapz_llm::Router>>,
    embedder: Arc<dyn recapz_core::embed::Embedder>,
    /// Object store holding finished recordings; `None` keeps them on local disk.
    store: Option<Arc<crate::storage::S3Store>>,
    /// When each member last asked, newest last; pruned to the last minute on every check.
    // ponytail: in memory and per process; a restart resets it, which is fine for one server.
    asks: Arc<
        Mutex<std::collections::HashMap<String, std::collections::VecDeque<std::time::Instant>>>,
    >,
    /// One-time sign-in codes for the desktop app.
    desktop_codes: Arc<desktop_auth::Codes>,
}

mod ask;
mod connect;
mod dev_live;
mod documents;
mod live;
mod live_ask;
mod record;

pub fn router(cfg: Config, control: Arc<Mutex<Connection>>) -> Router {
    let llm = crate::report_model::router_from_config(&cfg.llm)
        .unwrap_or_else(|e| {
            tracing::error!("llm config: {e:#}");
            None
        })
        .map(Arc::new);
    router_with_llm(cfg, control, llm)
}

pub fn router_with_llm(
    cfg: Config,
    control: Arc<Mutex<Connection>>,
    llm: Option<Arc<recapz_llm::Router>>,
) -> Router {
    let embedder =
        crate::embedder::from_config(&cfg.embedder, &cfg.llm).expect("embedder checked at start");
    let store =
        crate::storage::S3Store::from_config(&cfg.storage, tokio::runtime::Handle::current())
            .unwrap_or_else(|e| {
                tracing::error!("storage config: {e:#}");
                None
            })
            .map(Arc::new);
    let app = App {
        store,
        embedder,
        cfg: Arc::new(cfg),
        control,
        llm,
        asks: Default::default(),
        desktop_codes: Default::default(),
    };
    Router::new()
        .route(
            "/api/health",
            get(|| async { Json(serde_json::json!({ "status": "ok" })) }),
        )
        .route("/auth/google", get(google_start))
        .route("/auth/callback", get(google_callback))
        .route("/auth/desktop", get(desktop_auth::start))
        .route("/auth/desktop/exchange", post(desktop_auth::exchange))
        .route("/auth/calendar", get(connect::start))
        .route("/auth/calendar/callback", get(connect::callback))
        .route(
            "/api/calendar",
            get(connect::status).delete(connect::disconnect),
        )
        .route("/api/settings", get(get_settings).put(put_settings))
        .route(
            "/api/settings/audio-retention",
            get(get_audio_retention).put(put_audio_retention),
        )
        .route("/api/usage", get(get_usage))
        .route("/auth/dev-login", post(dev_login))
        .route("/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/meetings", get(list_meetings))
        .route("/api/meetings/upcoming", get(calendar::upcoming))
        .route("/api/meetings/join", post(calendar::join_link))
        .route("/api/calendar-events/{id}", patch(calendar::set_join))
        .route(
            "/api/meetings/{id}",
            get(meeting_detail)
                .patch(rename_meeting)
                .delete(delete_meeting),
        )
        .route("/api/meetings/{id}/transcript", get(meeting_transcript))
        .route("/api/meetings/{id}/report", get(meeting_report))
        .route("/api/meetings/{id}/audio", get(meeting_audio))
        .route("/api/meetings/{id}/live", get(live::live))
        .route("/api/meetings/{id}/record", get(record::socket))
        .route("/api/recordings", post(record::start))
        .route("/api/dev/live-sim", post(dev_live::start))
        .route(
            "/api/dev/live-sim/audio",
            post(dev_live::start_audio).layer(DefaultBodyLimit::max(dev_live::AUDIO_BODY_LIMIT)),
        )
        .route(
            "/api/dev/live-sim/{id}",
            axum::routing::delete(dev_live::stop),
        )
        .route("/api/meetings/{id}/live-ask", post(live_ask::live_ask))
        .route("/api/ask", post(ask::ask))
        .route(
            "/api/documents",
            get(documents::list)
                .merge(post(documents::upload).layer(DefaultBodyLimit::max(documents::BODY_LIMIT))),
        )
        .route("/api/documents/usage", get(documents::usage))
        .route(
            "/api/documents/{id}",
            patch(documents::set_llm_ok).delete(documents::delete),
        )
        .route(
            "/api/uploads",
            post(upload).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .fallback(crate::web::serve)
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            csrf_guard,
        ))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(app)
}

/// Double-submit CSRF check on every state-changing request that carries a session: the
/// `x-csrf-token` header must equal the `recapz_csrf` cookie, and a browser `Origin`, when sent,
/// must be this site. Requests without a session cookie have nothing to forge and fall through
/// to the usual 401 (sign-in itself is one of them).
// ponytail: the token is a random cookie value, not bound to the session id; bind it if sibling
// subdomains ever share this cookie jar.
async fn csrf_guard(
    State(app): State<App>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    use axum::http::Method;
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    let jar = CookieJar::from_headers(req.headers());
    if jar.get(SESSION_COOKIE).is_none() {
        return next.run(req).await;
    }
    let header_tok = req.headers().get(CSRF_HEADER).and_then(|v| v.to_str().ok());
    let token_ok = matches!((header_tok, jar.get(CSRF_COOKIE)),
        (Some(h), Some(c)) if !h.is_empty() && h == c.value());
    let origin_ok = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|o| o == app.cfg.base_url.trim_end_matches('/'));
    if token_ok && origin_ok {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "bad csrf token").into_response()
    }
}

/// Readable by the page script (not HttpOnly) so it can echo the value in `x-csrf-token`.
fn csrf_cookie(secure: bool) -> Cookie<'static> {
    Cookie::build((CSRF_COOKIE, db::new_id()))
        .path("/")
        .secure(secure)
        .same_site(SameSite::Lax)
        .max_age(time_secs(SESSION_TTL_MS))
        .build()
}

enum ApiError {
    Unauthorized,
    BadRequest(&'static str),
    Forbidden,
    Denied(&'static str),
    Conflict(&'static str),
    NotFound,
    Unavailable(&'static str),
    TooManyRequests,
    BadRange(u64),
    Internal(anyhow::Error),
}
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError::Internal(e.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, "sign in required").into_response()
            }
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "not invited").into_response(),
            ApiError::Denied(m) => (StatusCode::FORBIDDEN, m).into_response(),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m).into_response(),
            ApiError::NotFound => StatusCode::NOT_FOUND.into_response(),
            ApiError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, m).into_response(),
            ApiError::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "60")],
                "too many questions, try again in a minute",
            )
                .into_response(),
            ApiError::BadRange(len) => (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{len}"))],
            )
                .into_response(),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m).into_response(),
            ApiError::Internal(e) => {
                tracing::error!("{e:#}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}
type Res<T> = Result<T, ApiError>;

fn session_cookie(value: String, secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, value))
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .max_age(time_secs(SESSION_TTL_MS))
        .build()
}
fn time_secs(ms: i64) -> time::Duration {
    time::Duration::seconds(ms / 1000)
}

impl App {
    fn secure(&self) -> bool {
        self.cfg.base_url.starts_with("https://")
    }

    async fn control<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection) -> anyhow::Result<T> + Send + 'static,
    ) -> Res<T> {
        let c = self.control.clone();
        Ok(tokio::task::spawn_blocking(move || f(&c.lock().unwrap())).await??)
    }

    async fn tenant<T: Send + 'static>(
        &self,
        tenant_id: String,
        f: impl FnOnce(&Connection) -> anyhow::Result<T> + Send + 'static,
    ) -> Res<T> {
        let dir = self.cfg.data_dir.clone();
        Ok(tokio::task::spawn_blocking(move || f(&db::open_tenant(&dir, &tenant_id)?)).await??)
    }

    /// Counts one Ask for `member`; false when they already used their quota in the last minute.
    fn allow_ask(&self, member: &str) -> bool {
        let now = std::time::Instant::now();
        let mut all = self.asks.lock().unwrap();
        let q = all.entry(member.to_owned()).or_default();
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) >= std::time::Duration::from_secs(60))
        {
            q.pop_front();
        }
        if q.len() >= self.cfg.ask_per_minute as usize {
            return false;
        }
        q.push_back(now);
        true
    }

    async fn current_user(&self, jar: &CookieJar) -> Res<db::User> {
        let sid = jar
            .get(SESSION_COOKIE)
            .ok_or(ApiError::Unauthorized)?
            .value()
            .to_owned();
        self.control(move |c| db::user_for_session(c, &sid))
            .await?
            .ok_or(ApiError::Unauthorized)
    }

    async fn sign_in(&self, jar: CookieJar, email: String, name: Option<String>) -> Res<CookieJar> {
        let secure = self.secure();
        let sid = self
            .control(move |c| {
                let Some(u) = db::user_for_sign_in(c, &email, name.as_deref())? else {
                    return Ok(None);
                };
                Ok(Some(db::create_session(c, &u.id, SESSION_TTL_MS)?))
            })
            .await?
            .ok_or(ApiError::Forbidden)?;
        Ok(jar
            .add(session_cookie(sid, secure))
            .add(csrf_cookie(secure)))
    }
}

// ---- auth ----

async fn google_start(State(app): State<App>, jar: CookieJar) -> Res<(CookieJar, Redirect)> {
    if app.cfg.google.client_id.is_empty() {
        return Err(ApiError::BadRequest("Google sign-in is not configured"));
    }
    let state = db::new_id();
    let url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope=openid%20email%20profile&state={}",
        app.cfg.google.auth_url,
        enc(&app.cfg.google.client_id),
        enc(&format!("{}/auth/callback", app.cfg.base_url)),
        state
    );
    let c = Cookie::build((OAUTH_STATE_COOKIE, state))
        .path("/auth")
        .http_only(true)
        .secure(app.secure())
        .same_site(SameSite::Lax)
        .build();
    // A desktop sign-in that was abandoned must not turn this web sign-in into a desktop one;
    // `desktop_auth::start` adds the flag again after calling this.
    let jar = jar.remove(
        Cookie::build(desktop_auth::FLAG_COOKIE)
            .path("/auth")
            .build(),
    );
    Ok((jar.add(c), Redirect::to(&url)))
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: String,
    state: String,
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}
#[derive(Deserialize)]
struct UserInfo {
    email: String,
    #[serde(default)]
    email_verified: bool,
    name: Option<String>,
}

async fn google_callback(
    State(app): State<App>,
    jar: CookieJar,
    axum::extract::Query(q): axum::extract::Query<CallbackQuery>,
) -> Res<(CookieJar, Redirect)> {
    let expected = jar.get(OAUTH_STATE_COOKIE).map(|c| c.value().to_owned());
    if expected.as_deref() != Some(q.state.as_str()) {
        return Err(ApiError::BadRequest("invalid sign-in state"));
    }
    let http = reqwest::Client::new();
    let tok: TokenResponse = http
        .post(&app.cfg.google.token_url)
        .form(&[
            ("code", q.code.as_str()),
            ("client_id", &app.cfg.google.client_id),
            ("client_secret", &app.cfg.google.client_secret),
            (
                "redirect_uri",
                &format!("{}/auth/callback", app.cfg.base_url),
            ),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let info: UserInfo = http
        .get(&app.cfg.google.userinfo_url)
        .bearer_auth(tok.access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if !info.email_verified {
        return Err(ApiError::Forbidden);
    }
    if jar.get(desktop_auth::FLAG_COOKIE).is_some() {
        return desktop_auth::finish(&app, jar, info.email, info.name).await;
    }
    let jar = jar.remove(Cookie::build(OAUTH_STATE_COOKIE).path("/auth").build());
    let jar = app.sign_in(jar, info.email, info.name).await?;
    Ok((jar, Redirect::to("/")))
}

#[derive(Deserialize)]
struct DevLogin {
    email: String,
}

async fn dev_login(
    State(app): State<App>,
    jar: CookieJar,
    Json(b): Json<DevLogin>,
) -> Res<CookieJar> {
    if !app.cfg.dev_login {
        return Err(ApiError::Forbidden);
    }
    app.sign_in(jar, b.email, None).await
}

async fn logout(State(app): State<App>, jar: CookieJar) -> Res<CookieJar> {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        let sid = c.value().to_owned();
        app.control(move |c| db::delete_session(c, &sid)).await?;
    }
    Ok(jar
        .remove(Cookie::build(SESSION_COOKIE).path("/").build())
        .remove(Cookie::build(CSRF_COOKIE).path("/").build()))
}

// ---- api ----

#[derive(Serialize)]
struct Me {
    email: String,
    name: Option<String>,
    role: String,
    /// Where finished recordings are kept: `object_storage` or `local_disk`.
    recording_storage: &'static str,
}

async fn me(State(app): State<App>, jar: CookieJar) -> Res<(CookieJar, Json<Me>)> {
    let u = app.current_user(&jar).await?;
    // Sessions that began before CSRF tokens existed get theirs on the next page load.
    let jar = if jar.get(CSRF_COOKIE).is_none() {
        jar.add(csrf_cookie(app.secure()))
    } else {
        jar
    };
    Ok((
        jar,
        Json(Me {
            email: u.email,
            name: u.name,
            role: u.role,
            recording_storage: if app.store.is_some() {
                "object_storage"
            } else {
                "local_disk"
            },
        }),
    ))
}

#[derive(Serialize, serde::Deserialize)]
struct AudioRetention {
    /// Days finished recordings keep their audio; 0 keeps it forever.
    days: u32,
}

async fn get_audio_retention(State(app): State<App>, jar: CookieJar) -> Res<Json<AudioRetention>> {
    let u = app.current_user(&jar).await?;
    let days = app
        .tenant(u.tenant_id.clone(), |c| {
            recapz_core::settings::audio_retention_days(c)
        })
        .await?;
    Ok(Json(AudioRetention { days }))
}

/// Admin only. Takes effect at the next hourly sweep.
async fn put_audio_retention(
    State(app): State<App>,
    jar: CookieJar,
    Json(body): Json<AudioRetention>,
) -> Res<Json<AudioRetention>> {
    let u = app.current_user(&jar).await?;
    if u.role != "admin" {
        return Err(ApiError::Denied("only an admin can change retention"));
    }
    app.tenant(u.tenant_id.clone(), move |c| {
        Ok(
            recapz_core::settings::set_audio_retention_days(c, body.days)
                .map_err(|_| ApiError::BadRequest("retention is at most 3650 days")),
        )
    })
    .await??;
    Ok(Json(AudioRetention { days: body.days }))
}

#[derive(Serialize, serde::Deserialize)]
struct JoinSettings {
    join_rule: String,
    notetaker_name: String,
    join_message: String,
}

async fn get_settings(State(app): State<App>, jar: CookieJar) -> Res<Json<JoinSettings>> {
    let u = app.current_user(&jar).await?;
    let out = app
        .tenant(u.tenant_id.clone(), |c| {
            use recapz_core::settings as s;
            Ok(JoinSettings {
                join_rule: s::join_rule(c)?.as_str().to_owned(),
                notetaker_name: s::notetaker_name(c)?,
                join_message: s::join_message(c)?,
            })
        })
        .await?;
    Ok(Json(out))
}

/// Admin only. Replaces all three values; a bad one changes nothing.
async fn put_settings(
    State(app): State<App>,
    jar: CookieJar,
    Json(b): Json<JoinSettings>,
) -> Res<Json<JoinSettings>> {
    let u = app.current_user(&jar).await?;
    if u.role != "admin" {
        return Err(ApiError::Denied("only an admin can change settings"));
    }
    let (rule, name, msg) = (
        b.join_rule.clone(),
        b.notetaker_name.clone(),
        b.join_message.clone(),
    );
    app.tenant(u.tenant_id.clone(), move |c| {
        Ok(
            recapz_core::settings::set_join_settings(c, &rule, &name, &msg)
                .map(|r| r.map_err(ApiError::BadRequest)),
        )
    })
    .await???;
    Ok(Json(b))
}

#[derive(Serialize)]
struct UsageMeeting {
    id: String,
    title: String,
    speech_micro_usd: i64,
    llm_micro_usd: i64,
}

#[derive(Serialize)]
struct UsageOut {
    /// Estimates for the current UTC month, in micro-USD (1_000_000 = $1).
    speech_micro_usd: i64,
    llm_micro_usd: i64,
    total_micro_usd: i64,
    /// 0 means no cap.
    cap_micro_usd: u64,
    /// True when the cap is reached: new paid jobs wait until next month or a higher cap.
    paused: bool,
    meetings: Vec<UsageMeeting>,
}

/// Admin only: this month's estimated cost and the cap (workspace totals), and the costliest meetings
/// the admin may see (own or shared with the workspace), like every other meeting read.
async fn get_usage(State(app): State<App>, jar: CookieJar) -> Res<Json<UsageOut>> {
    let u = app.current_user(&jar).await?;
    if u.role != "admin" {
        return Err(ApiError::Denied("only an admin can see usage"));
    }
    let cap = app.cfg.monthly_cap_micro_usd();
    let (uid, tenant) = (u.id.clone(), u.tenant_id.clone());
    let out = app
        .tenant(tenant, move |c| {
            let now = db::now_ms();
            let t = recapz_core::usage::month_totals(c, now)?;
            Ok(UsageOut {
                speech_micro_usd: t.speech_micro_usd,
                llm_micro_usd: t.llm_micro_usd,
                total_micro_usd: t.total(),
                cap_micro_usd: cap,
                paused: cap > 0 && t.total() >= cap as i64,
                meetings: recapz_core::usage::per_meeting(c, now, 20, &uid)?
                    .into_iter()
                    .map(|m| UsageMeeting {
                        id: m.meeting_id,
                        title: m.title,
                        speech_micro_usd: m.speech_micro_usd,
                        llm_micro_usd: m.llm_micro_usd,
                    })
                    .collect(),
            })
        })
        .await?;
    Ok(Json(out))
}

/// Makes sure the signed-in user has a members row in their tenant DB (meeting owner FK).
fn ensure_member(c: &Connection, u: &db::User) -> anyhow::Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO members (id, email, name, role) VALUES (?1, ?2, ?3, ?4)",
        params![u.id, u.email, u.name, u.role],
    )?;
    Ok(())
}

#[derive(Serialize)]
struct MeetingRow {
    id: String,
    title: String,
    platform: String,
    status: String,
    started_at: Option<i64>,
    created_at: i64,
    duration_ms: Option<i64>,
    action_item_count: i64,
    speakers: Vec<ListSpeaker>,
}

#[derive(Serialize)]
struct ListSpeaker {
    id: String,
    name: String,
}

const LIST_SPEAKER_CAP: i64 = 5;

async fn list_meetings(State(app): State<App>, jar: CookieJar) -> Res<Json<Vec<MeetingRow>>> {
    let u = app.current_user(&jar).await?;
    let uid = u.id.clone();
    let rows = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            // Visible to the owner, or to everyone when shared with the workspace.
            let mut s = c.prepare(
                "SELECT id, title, platform, status, started_at, created_at,
                        CASE WHEN started_at IS NOT NULL AND ended_at >= started_at THEN ended_at - started_at END,
                        (SELECT count(*) FROM action_items a WHERE a.meeting_id = meetings.id)
                 FROM meetings
                 WHERE owner_id = ?1 OR share = 'workspace' ORDER BY COALESCE(started_at, created_at) DESC",
            )?;
            let mut rows = s
                .query_map([&uid], |r| {
                    Ok(MeetingRow {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        platform: r.get(2)?,
                        status: r.get(3)?,
                        started_at: r.get(4)?,
                        created_at: r.get(5)?,
                        duration_ms: r.get(6)?,
                        action_item_count: r.get(7)?,
                        speakers: Vec::new(),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            // One query for every listed meeting's speakers, with the same visibility rule.
            let mut sp = c.prepare(
                "SELECT meeting_id, id, name FROM (
                   SELECT p.meeting_id, p.id, COALESCE(NULLIF(p.display_name, ''), p.label) AS name,
                          row_number() OVER (PARTITION BY p.meeting_id ORDER BY p.rowid) AS n
                   FROM speakers p JOIN meetings m ON m.id = p.meeting_id
                   WHERE m.owner_id = ?1 OR m.share = 'workspace') WHERE n <= ?2",
            )?;
            let mut by: std::collections::HashMap<String, Vec<ListSpeaker>> = Default::default();
            for x in sp.query_map(params![uid, LIST_SPEAKER_CAP], |r| {
                Ok((r.get::<_, String>(0)?, ListSpeaker { id: r.get(1)?, name: r.get(2)? }))
            })? {
                let (m, v) = x?;
                by.entry(m).or_default().push(v);
            }
            for r in &mut rows {
                r.speakers = by.remove(&r.id).unwrap_or_default();
            }
            Ok(rows)
        })
        .await?;
    Ok(Json(rows))
}

#[derive(Serialize)]
struct MeetingDetail {
    id: String,
    title: String,
    platform: String,
    status: String,
    language: String,
    started_at: Option<i64>,
    ended_at: Option<i64>,
    created_at: i64,
    error: Option<String>,
    /// Same order, cap and names as the list rows: the colour index every screen derives from.
    speakers: Vec<ListSpeaker>,
}

/// The one meeting access rule: the owner, or anyone when shared with the workspace.
/// A meeting the member may not see is reported as missing, so its existence does not leak.
fn visible_meeting(c: &Connection, uid: &str, id: &str) -> anyhow::Result<Option<MeetingDetail>> {
    Ok(c.query_row(
        "SELECT id, title, platform, status, language, started_at, ended_at, created_at, error
         FROM meetings WHERE id = ?1 AND (owner_id = ?2 OR share = 'workspace')",
        params![id, uid],
        |r| {
            Ok(MeetingDetail {
                id: r.get(0)?,
                title: r.get(1)?,
                platform: r.get(2)?,
                status: r.get(3)?,
                language: r.get(4)?,
                started_at: r.get(5)?,
                ended_at: r.get(6)?,
                created_at: r.get(7)?,
                error: r.get(8)?,
                speakers: Vec::new(),
            })
        },
    )
    .optional()?)
}

async fn meeting_detail(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<Json<MeetingDetail>> {
    let u = app.current_user(&jar).await?;
    let uid = u.id.clone();
    let m = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            // Access is decided by visible_meeting; the speakers query only runs for a visible meeting.
            let Some(mut m) = visible_meeting(c, &uid, &id)? else {
                return Ok(None);
            };
            let mut sp = c.prepare(
                "SELECT id, COALESCE(NULLIF(display_name, ''), label) FROM speakers
                 WHERE meeting_id = ?1 ORDER BY rowid LIMIT ?2",
            )?;
            m.speakers = sp
                .query_map(params![id, LIST_SPEAKER_CAP], |r| {
                    Ok(ListSpeaker {
                        id: r.get(0)?,
                        name: r.get(1)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            Ok(Some(m))
        })
        .await?;
    Ok(Json(m.ok_or(ApiError::NotFound)?))
}

const MAX_TITLE_CHARS: usize = 200;

#[derive(Deserialize)]
struct Rename {
    title: String,
}

/// Owner or admin only, like delete. Allowed in any status: a title is not part of processing.
async fn rename_meeting(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(body): Json<Rename>,
) -> Res<Json<serde_json::Value>> {
    let title = body.title.trim().to_string();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return Err(ApiError::BadRequest("title must be 1 to 200 characters"));
    }
    let u = app.current_user(&jar).await?;
    let saved = title.clone();
    app.tenant(u.tenant_id.clone(), move |c| {
        ensure_member(c, &u)?;
        if visible_meeting(c, &u.id, &id)?.is_none() {
            return Ok(Err(ApiError::NotFound));
        }
        let owner: Option<String> =
            c.query_row("SELECT owner_id FROM meetings WHERE id = ?1", [&id], |r| {
                r.get(0)
            })?;
        if owner.as_deref() != Some(u.id.as_str()) && u.role != "admin" {
            return Ok(Err(ApiError::Denied(
                "only the owner or an admin can rename",
            )));
        }
        c.execute(
            "UPDATE meetings SET title = ?1 WHERE id = ?2",
            params![title, id],
        )?;
        Ok(Ok(()))
    })
    .await??;
    Ok(Json(serde_json::json!({ "title": saved })))
}

/// Hard delete. Owner or admin only, and not while the meeting is still being captured or processed.
async fn delete_meeting(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<StatusCode> {
    let u = app.current_user(&jar).await?;
    let (dir, tenant) = (app.cfg.data_dir.clone(), u.tenant_id.clone());
    let store = app.store.clone();
    app.tenant(tenant.clone(), move |c| {
        ensure_member(c, &u)?;
        let Some(m) = visible_meeting(c, &u.id, &id)? else {
            return Ok(Err(ApiError::NotFound));
        };
        let owner: Option<String> =
            c.query_row("SELECT owner_id FROM meetings WHERE id = ?1", [&id], |r| {
                r.get(0)
            })?;
        if owner.as_deref() != Some(u.id.as_str()) && u.role != "admin" {
            return Ok(Err(ApiError::Denied(
                "only the owner or an admin can delete",
            )));
        }
        if matches!(
            m.status.as_str(),
            "joining" | "recording" | "transcribing" | "reporting"
        ) {
            return Ok(Err(ApiError::Conflict("meeting is still processing")));
        }
        if purge_meeting(
            c,
            &dir,
            &tenant,
            &id,
            store
                .as_deref()
                .map(|s| s as &dyn recapz_core::blobs::Blobs),
        )? {
            tracing::info!(tenant = %tenant, meeting = %id, member = %u.id, "meeting deleted");
        }
        Ok(Ok(()))
    })
    .await??;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct SegmentRow {
    id: i64,
    speaker_id: Option<String>,
    speaker_name: Option<String>,
    start_ms: i64,
    end_ms: i64,
    text: String,
}

async fn meeting_transcript(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<Json<Vec<SegmentRow>>> {
    let u = app.current_user(&jar).await?;
    let uid = u.id.clone();
    let rows = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            if visible_meeting(c, &uid, &id)?.is_none() {
                return Ok(None);
            }
            let mut s = c.prepare(
                "SELECT g.id, g.speaker_id, COALESCE(p.display_name, p.label), g.start_ms, g.end_ms, g.text
                 FROM segments g LEFT JOIN speakers p ON p.id = g.speaker_id
                 WHERE g.meeting_id = ?1 ORDER BY g.start_ms, g.id",
            )?;
            let rows = s
                .query_map([&id], |r| {
                    Ok(SegmentRow {
                        id: r.get(0)?,
                        speaker_id: r.get(1)?,
                        speaker_name: r.get(2)?,
                        start_ms: r.get(3)?,
                        end_ms: r.get(4)?,
                        text: r.get(5)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(rows))
        })
        .await?;
    Ok(Json(rows.ok_or(ApiError::NotFound)?))
}

#[derive(Serialize)]
struct ReportOut {
    summary: String,
    decisions: Vec<String>,
    open_questions: Vec<String>,
    chapters: Vec<serde_json::Value>,
    action_items: Vec<ActionItemOut>,
    model: Option<String>,
    created_at: Option<i64>,
}

#[derive(Serialize)]
struct ActionItemOut {
    id: String,
    text: String,
    due_date: Option<String>,
    done: bool,
    source_ms: Option<i64>,
}

fn json_list<T: serde::de::DeserializeOwned>(raw: Option<String>) -> Vec<T> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// The meeting report, or `null` while it has not been written yet.
async fn meeting_report(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<Json<Option<ReportOut>>> {
    let u = app.current_user(&jar).await?;
    let uid = u.id.clone();
    let out = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            if visible_meeting(c, &uid, &id)?.is_none() {
                return Ok(None);
            }
            let report = c
                .query_row(
                    "SELECT summary, decisions_json, questions_json, chapters_json, model, created_at
                     FROM reports WHERE meeting_id = ?1",
                    [&id],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, Option<String>>(4)?,
                            r.get::<_, Option<i64>>(5)?,
                        ))
                    },
                )
                .optional()?;
            let Some((summary, dec, q, ch, model, created_at)) = report else {
                return Ok(Some(None));
            };
            let mut s = c.prepare(
                "SELECT id, text, due_date, done, source_ms FROM action_items
                 WHERE meeting_id = ?1 ORDER BY source_ms, id",
            )?;
            let action_items = s
                .query_map([&id], |r| {
                    Ok(ActionItemOut {
                        id: r.get(0)?,
                        text: r.get(1)?,
                        due_date: r.get(2)?,
                        done: r.get::<_, i64>(3)? != 0,
                        source_ms: r.get(4)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(Some(ReportOut {
                summary: summary.unwrap_or_default(),
                decisions: json_list(dec),
                open_questions: json_list(q),
                chapters: json_list(ch),
                action_items,
                model,
                created_at,
            })))
        })
        .await?;
    Ok(Json(out.ok_or(ApiError::NotFound)?))
}

/// One `bytes=start-end`, `bytes=start-` or `bytes=-suffix` range, clamped to the file.
/// ponytail: multi-range requests are treated as unsatisfiable; players only send one.
fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let (a, b) = h.strip_prefix("bytes=")?.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => (len.checked_sub(n.parse().ok()?)?, len.checked_sub(1)?),
        (a, "") => (a.parse().ok()?, len.checked_sub(1)?),
        (a, b) => (
            a.parse().ok()?,
            b.parse::<u64>().ok()?.min(len.checked_sub(1)?),
        ),
    };
    (start <= end && end < len).then_some((start, end))
}

fn audio_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("m4a" | "mp4") => "audio/mp4",
        Some("ogg") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("webm") => "audio/webm",
        _ => "application/octet-stream",
    }
}

async fn meeting_audio(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Res<Response> {
    let u = app.current_user(&jar).await?;
    let (uid, tenant) = (u.id.clone(), u.tenant_id.clone());
    let (rel, remote) = app
        .tenant(tenant.clone(), move |c| {
            ensure_member(c, &u)?;
            if visible_meeting(c, &uid, &id)?.is_none() {
                return Ok(None);
            }
            let (path, remote) = c.query_row(
                "SELECT audio_path, audio_remote FROM meetings WHERE id = ?1",
                [&id],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, bool>(1)?)),
            )?;
            Ok(path.map(|p| (p, remote)))
        })
        .await?
        .ok_or(ApiError::NotFound)?;
    if remote {
        return remote_audio(&app, &tenant, &rel, &headers).await;
    }
    let mut f = tokio::fs::File::open(db::tenant_dir(&app.cfg.data_dir, &tenant).join(&rel))
        .await
        .map_err(|_| ApiError::NotFound)?;
    let len = f.metadata().await?.len();
    let range = match headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        None => None,
        Some(h) => Some(parse_range(h, len).ok_or(ApiError::BadRange(len))?),
    };
    let (start, end) = range.unwrap_or((0, len.saturating_sub(1)));
    f.seek(std::io::SeekFrom::Start(start)).await?;
    // Streamed, so a multi-gigabyte recording is never held in memory.
    let n = if len == 0 { 0 } else { end - start + 1 };
    let body = Body::from_stream(ReaderStream::new(f.take(n)));
    let mut res = (
        if range.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        },
        [
            (header::CONTENT_TYPE, audio_type(&rel).to_string()),
            (header::ACCEPT_RANGES, "bytes".to_string()),
        ],
        body,
    )
        .into_response();
    if range.is_some() {
        res.headers_mut().insert(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{len}").parse().unwrap(),
        );
    }
    Ok(res)
}

/// Largest piece fetched from the bucket for one ranged request. A player asking `bytes=0-` for a
/// whole recording gets this much with 206 and asks again for the rest.
const REMOTE_CHUNK: u64 = 8 * 1024 * 1024;

/// Plays a recording that lives in the object store: ranged requests become ranged GETs, a plain
/// request streams the whole object. The bucket is private; this is the only way to reach it.
async fn remote_audio(app: &App, tenant: &str, rel: &str, headers: &HeaderMap) -> Res<Response> {
    let store = app
        .store
        .as_ref()
        .ok_or(ApiError::Unavailable("recording storage is not configured"))?;
    let key = recapz_core::blobs::audio_key(tenant, rel);
    let len = store.size(&key).await?.ok_or(ApiError::NotFound)?;
    let ctype = [
        (header::CONTENT_TYPE, audio_type(rel).to_string()),
        (header::ACCEPT_RANGES, "bytes".to_string()),
    ];
    let Some(h) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) else {
        let body = Body::from_stream(store.stream(&key).await?);
        return Ok((StatusCode::OK, ctype, body).into_response());
    };
    let (start, end) = parse_range(h, len).ok_or(ApiError::BadRange(len))?;
    let end = end.min(start + REMOTE_CHUNK - 1);
    let bytes = store.range(&key, start, end).await?;
    let mut res = (StatusCode::PARTIAL_CONTENT, ctype, bytes).into_response();
    res.headers_mut().insert(
        header::CONTENT_RANGE,
        format!("bytes {start}-{end}/{len}").parse().unwrap(),
    );
    Ok(res)
}

#[derive(Serialize)]
struct Created {
    id: String,
}

fn safe_name(n: &str) -> String {
    let s: String = n
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    s.trim_start_matches('.').chars().take(100).collect()
}

/// The speech language a member picks for a meeting: `vi` (the default), `en` or `multi` (mixed
/// calls). Anything else is a bad request, never silently turned into another language.
fn meeting_language(raw: Option<&str>) -> Result<&'static str, ApiError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("vi") => Ok("vi"),
        Some("en") => Ok("en"),
        Some("multi") => Ok("multi"),
        Some(_) => Err(ApiError::BadRequest("language must be vi, en or multi")),
    }
}

async fn upload(
    State(app): State<App>,
    jar: CookieJar,
    mut mp: Multipart,
) -> Res<(StatusCode, Json<Created>)> {
    let u = app.current_user(&jar).await?;
    let mut title = None;
    let mut language = None;
    let mut file: Option<(String, axum::body::Bytes)> = None;
    while let Some(f) = mp
        .next_field()
        .await
        .map_err(|_| ApiError::BadRequest("bad multipart body"))?
    {
        match f.name() {
            Some("title") => title = f.text().await.ok().filter(|t| !t.trim().is_empty()),
            Some("language") => language = f.text().await.ok(),
            Some("file") => {
                let name = f.file_name().unwrap_or("upload").to_owned();
                let bytes = f
                    .bytes()
                    .await
                    .map_err(|_| ApiError::BadRequest("could not read file"))?;
                file = Some((name, bytes));
            }
            _ => {}
        }
    }
    let language = meeting_language(language.as_deref())?;
    let (name, bytes) = file.ok_or(ApiError::BadRequest("missing file field"))?;
    if bytes.is_empty() {
        return Err(ApiError::BadRequest("empty file"));
    }
    let id = db::new_id();
    let rel = format!("audio/{id}/{}", safe_name(&name));
    let path = db::tenant_dir(&app.cfg.data_dir, &u.tenant_id).join(&rel);
    tokio::fs::create_dir_all(path.parent().unwrap()).await?;
    tokio::fs::write(&path, &bytes).await?;

    let title = title.unwrap_or_else(|| name.clone());
    let (mid, rel2) = (id.clone(), rel.clone());
    let res = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            c.execute(
                "INSERT INTO meetings (id, title, platform, status, owner_id, audio_path, language, created_at)
                 VALUES (?1, ?2, 'upload', 'uploaded', ?3, ?4, ?5, ?6)",
                params![mid, title, u.id, rel2, language, db::now_ms()],
            )?;
            jobs::enqueue(c, "transcribe", Some(&mid), None)?;
            Ok(())
        })
        .await;
    if res.is_err() {
        let _ = tokio::fs::remove_dir_all(path.parent().unwrap()).await; // no orphan audio on failure
    }
    res?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}

#[cfg(test)]
mod api_error_tests {
    use super::*;

    #[tokio::test]
    async fn an_internal_error_never_reaches_the_client() {
        let e = anyhow::anyhow!("GET http://10.0.0.5/v1/embeddings failed");
        let r = ApiError::Internal(e).into_response();
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let b = axum::body::to_bytes(r.into_body(), 1024).await.unwrap();
        assert_eq!(b.len(), 0);
    }
}
