use crate::config::Config;
use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use recapz_core::{db, jobs};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

const SESSION_COOKIE: &str = "recapz_session";
const OAUTH_STATE_COOKIE: &str = "recapz_oauth_state";
const SESSION_TTL_MS: i64 = 30 * 24 * 3600 * 1000;
const MAX_UPLOAD_BYTES: usize = 2 * 1024 * 1024 * 1024;

#[derive(Clone)]
pub struct App {
    cfg: Arc<Config>,
    // ponytail: one shared control connection behind a mutex; fine for one team, pool if it ever contends.
    control: Arc<Mutex<Connection>>,
}

pub fn router(cfg: Config, control: Arc<Mutex<Connection>>) -> Router {
    let app = App {
        cfg: Arc::new(cfg),
        control,
    };
    Router::new()
        .route(
            "/api/health",
            get(|| async { Json(serde_json::json!({ "status": "ok" })) }),
        )
        .route("/auth/google", get(google_start))
        .route("/auth/callback", get(google_callback))
        .route("/auth/dev-login", post(dev_login))
        .route("/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/meetings", get(list_meetings))
        .route(
            "/api/uploads",
            post(upload).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(app)
}

enum ApiError {
    Unauthorized,
    BadRequest(&'static str),
    Forbidden,
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
        Ok(jar.add(session_cookie(sid, secure)))
    }
}

// ---- auth ----

async fn google_start(State(app): State<App>, jar: CookieJar) -> Res<(CookieJar, Redirect)> {
    if app.cfg.google.client_id.is_empty() {
        return Err(ApiError::BadRequest("Google sign-in is not configured"));
    }
    let state = db::new_id();
    let url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope=openid%20email%20profile&state={}",
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
        .post("https://oauth2.googleapis.com/token")
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
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(tok.access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if !info.email_verified {
        return Err(ApiError::Forbidden);
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
    Ok(jar.remove(Cookie::build(SESSION_COOKIE).path("/").build()))
}

// ---- api ----

#[derive(Serialize)]
struct Me {
    email: String,
    name: Option<String>,
    role: String,
}

async fn me(State(app): State<App>, jar: CookieJar) -> Res<Json<Me>> {
    let u = app.current_user(&jar).await?;
    Ok(Json(Me {
        email: u.email,
        name: u.name,
        role: u.role,
    }))
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
}

async fn list_meetings(State(app): State<App>, jar: CookieJar) -> Res<Json<Vec<MeetingRow>>> {
    let u = app.current_user(&jar).await?;
    let uid = u.id.clone();
    let rows = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            // Visible to the owner, or to everyone when shared with the workspace.
            let mut s = c.prepare(
                "SELECT id, title, platform, status, started_at, created_at FROM meetings
                 WHERE owner_id = ?1 OR share = 'workspace' ORDER BY COALESCE(started_at, created_at) DESC",
            )?;
            let rows = s
                .query_map([uid], |r| {
                    Ok(MeetingRow {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        platform: r.get(2)?,
                        status: r.get(3)?,
                        started_at: r.get(4)?,
                        created_at: r.get(5)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(rows))
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

async fn upload(
    State(app): State<App>,
    jar: CookieJar,
    mut mp: Multipart,
) -> Res<(StatusCode, Json<Created>)> {
    let u = app.current_user(&jar).await?;
    let mut title = None;
    let mut file: Option<(String, axum::body::Bytes)> = None;
    while let Some(f) = mp
        .next_field()
        .await
        .map_err(|_| ApiError::BadRequest("bad multipart body"))?
    {
        match f.name() {
            Some("title") => title = f.text().await.ok().filter(|t| !t.trim().is_empty()),
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
                "INSERT INTO meetings (id, title, platform, status, owner_id, audio_path, created_at)
                 VALUES (?1, ?2, 'upload', 'uploaded', ?3, ?4, ?5)",
                params![mid, title, u.id, rel2, db::now_ms()],
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
