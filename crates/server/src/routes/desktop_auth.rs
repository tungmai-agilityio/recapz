//! Sign-in for the desktop app: Google runs in the system browser (Google blocks embedded webviews), the
//! server hands back `recapz://signed-in?code=…`, and the app's webview swaps that one-time code for the
//! normal session cookie.
use super::{
    csrf_cookie, enc, session_cookie, ApiError, App, Res, OAUTH_STATE_COOKIE, SESSION_TTL_MS,
};
use axum::{
    extract::State,
    http::{header, HeaderMap},
    response::Redirect,
    Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use recapz_core::db;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::time::Instant;

/// Marks a sign-in that started in the desktop app, so the Google callback ends in the app's deep link.
pub(super) const FLAG_COOKIE: &str = "recapz_desktop";
const CODE_TTL: Duration = Duration::from_secs(60);

/// One-time codes by SHA-256 of the code: the member and when the code expires.
// ponytail: in memory and per process; a restart drops codes still inside their 60 s, and the member just
// signs in again. Move to the control database if the server ever runs as more than one process.
pub(super) type Codes = std::sync::Mutex<std::collections::HashMap<String, (String, Instant)>>;

fn hash(code: &str) -> String {
    Sha256::digest(code.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl App {
    fn mint_desktop_code(&self, user_id: String) -> String {
        let code = format!("{}{}", db::new_id(), db::new_id()).replace('-', "");
        let now = Instant::now();
        let mut codes = self.desktop_codes.lock().unwrap();
        codes.retain(|_, (_, exp)| *exp > now);
        codes.insert(hash(&code), (user_id, now + CODE_TTL));
        code
    }

    /// Removes the code whether or not it is still valid, so it can never be tried twice.
    fn take_desktop_code(&self, code: &str) -> Option<String> {
        let (user_id, exp) = self.desktop_codes.lock().unwrap().remove(&hash(code))?;
        (exp > Instant::now()).then_some(user_id)
    }
}

/// `GET /auth/desktop`: the normal Google sign-in, remembered as a desktop one.
pub(super) async fn start(State(app): State<App>, jar: CookieJar) -> Res<(CookieJar, Redirect)> {
    let flag = Cookie::build((FLAG_COOKIE, "1"))
        .path("/auth")
        .http_only(true)
        .secure(app.secure())
        .same_site(SameSite::Lax)
        .max_age(time::Duration::minutes(10))
        .build();
    let (jar, to) = super::google_start(State(app), jar).await?;
    Ok((jar.add(flag), to))
}

/// Ends a desktop sign-in after Google vouched for the email: no session in the browser, only a code.
pub(super) async fn finish(
    app: &App,
    jar: CookieJar,
    email: String,
    name: Option<String>,
) -> Res<(CookieJar, Redirect)> {
    let user = app
        .control(move |c| db::user_for_sign_in(c, &email, name.as_deref()))
        .await?
        .ok_or(ApiError::Forbidden)?;
    let code = app.mint_desktop_code(user.id);
    let jar = jar
        .remove(Cookie::build(OAUTH_STATE_COOKIE).path("/auth").build())
        .remove(Cookie::build(FLAG_COOKIE).path("/auth").build());
    Ok((
        jar,
        Redirect::to(&format!("recapz://signed-in?code={}", enc(&code))),
    ))
}

#[derive(Deserialize)]
pub(super) struct Exchange {
    code: String,
}

/// `POST /auth/desktop/exchange`: the code for the usual session cookie. The request must come from this
/// site's own origin (the app's webview), and a stale session cookie still goes through the CSRF guard.
pub(super) async fn exchange(
    State(app): State<App>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(b): Json<Exchange>,
) -> Res<CookieJar> {
    let same_origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| o == app.cfg.base_url.trim_end_matches('/'));
    if !same_origin {
        return Err(ApiError::Denied("wrong origin"));
    }
    let user_id = app
        .take_desktop_code(&b.code)
        .ok_or(ApiError::Denied("sign-in code is invalid or expired"))?;
    let sid = app
        .control(move |c| db::create_session(c, &user_id, SESSION_TTL_MS))
        .await?;
    let secure = app.secure();
    Ok(jar
        .add(session_cookie(sid, secure))
        .add(csrf_cookie(secure)))
}
