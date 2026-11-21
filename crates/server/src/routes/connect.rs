//! Connect and disconnect Google Calendar for the signed-in member. Sign-in keeps its own small
//! scopes; this is a separate, optional grant (calendar read-only, offline) whose refresh token is
//! stored encrypted. Nothing is stored when the member unticks the calendar scope.
use super::{enc, ensure_member, ApiError, App, Res, UserInfo};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Redirect,
    Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use recapz_core::{db, google_tokens};
use serde::Deserialize;

/// Not the sign-in state cookie, so one flow's state is never accepted by the other.
const STATE_COOKIE: &str = "recapz_cal_state";
const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events.readonly";

fn redirect_uri(app: &App) -> String {
    format!("{}/auth/calendar/callback", app.cfg.base_url)
}

fn key(app: &App) -> Res<google_tokens::SecretKey> {
    if app.cfg.google.client_id.is_empty() {
        return Err(ApiError::Unavailable("Google is not configured"));
    }
    google_tokens::SecretKey::from_hex(&app.cfg.secret_key)
        .map_err(|_| ApiError::Unavailable("calendar connection needs RECAPZ_SECRET_KEY"))
}

pub(super) async fn start(State(app): State<App>, jar: CookieJar) -> Res<(CookieJar, Redirect)> {
    app.current_user(&jar).await?;
    key(&app)?;
    let state = db::new_id();
    // `openid email` lets the callback check the grant came from this member's own account.
    let url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&include_granted_scopes=true&state={}",
        app.cfg.google.auth_url,
        enc(&app.cfg.google.client_id),
        enc(&redirect_uri(&app)),
        enc(&format!("openid email {CALENDAR_SCOPE}")),
        state
    );
    let c = Cookie::build((STATE_COOKIE, state))
        .path("/auth")
        .http_only(true)
        .secure(app.secure())
        .same_site(SameSite::Lax)
        .build();
    Ok((jar.add(c), Redirect::to(&url)))
}

#[derive(Deserialize)]
pub(super) struct Callback {
    code: Option<String>,
    error: Option<String>,
    state: String,
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

/// Asks Google to cancel a grant. Best effort: a failure is logged without the token.
async fn revoke(app: &App, member: &str, token: &str) {
    let sent = reqwest::Client::new()
        .post(&app.cfg.google.revoke_url)
        .form(&[("token", token)])
        .send()
        .await;
    if sent.is_err() {
        tracing::warn!(member = %member, "Google revoke failed");
    }
}

/// Ends the flow by sending the member back to the app with the outcome in the query.
fn back(jar: CookieJar, outcome: &str) -> Res<(CookieJar, Redirect)> {
    let jar = jar.remove(Cookie::build(STATE_COOKIE).path("/auth").build());
    Ok((jar, Redirect::to(&format!("/recording?calendar={outcome}"))))
}

pub(super) async fn callback(
    State(app): State<App>,
    jar: CookieJar,
    Query(q): Query<Callback>,
) -> Res<(CookieJar, Redirect)> {
    let u = app.current_user(&jar).await?;
    let k = key(&app)?;
    let expected = jar.get(STATE_COOKIE).map(|c| c.value().to_owned());
    if expected.as_deref() != Some(q.state.as_str()) {
        return Err(ApiError::BadRequest("invalid calendar state"));
    }
    // Declined, or refused by Google (for example an account outside the Workspace domain).
    let Some(code) = q.code.filter(|_| q.error.is_none()) else {
        return back(jar, "denied");
    };
    let g = &app.cfg.google;
    let http = reqwest::Client::new();
    let tok: Tokens = http
        .post(&g.token_url)
        .form(&[
            ("code", code.as_str()),
            ("client_id", &g.client_id),
            ("client_secret", &g.client_secret),
            ("redirect_uri", &redirect_uri(&app)),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    // The member can untick calendar on the consent screen: then nothing is stored.
    // Either way Google already issued a grant we will not keep, so cancel it.
    let unwanted = tok.refresh_token.as_deref().unwrap_or(&tok.access_token);
    if !tok.scope.split(' ').any(|s| s == CALENDAR_SCOPE) {
        revoke(&app, &u.id, unwanted).await;
        return back(jar, "scope_missing");
    }
    let info: UserInfo = http
        .get(&g.userinfo_url)
        .bearer_auth(&tok.access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if !info.email_verified || !info.email.eq_ignore_ascii_case(&u.email) {
        revoke(&app, &u.id, unwanted).await;
        return back(jar, "wrong_account");
    }
    let Some(refresh) = tok.refresh_token else {
        return back(jar, "error");
    };
    let scope = tok.scope;
    let (tenant, member) = (u.tenant_id.clone(), u.id.clone());
    app.tenant(u.tenant_id.clone(), move |c| {
        ensure_member(c, &u)?;
        google_tokens::save(c, &k, &u.id, &refresh, &scope)
    })
    .await?;
    // First fetch now, so the member does not wait up to ten minutes for the loop to reach them.
    let (data_dir, google, k) = (app.cfg.data_dir.clone(), app.cfg.google.clone(), key(&app)?);
    tokio::spawn(async move {
        if let Err(e) =
            crate::calendar_sync::sync_member(&data_dir, &tenant, &member, &google, k, db::now_ms())
                .await
        {
            tracing::warn!(member = %member, "first calendar sync failed: {e:#}");
        }
    });
    back(jar, "connected")
}

pub(super) async fn status(State(app): State<App>, jar: CookieJar) -> Res<Json<serde_json::Value>> {
    let u = app.current_user(&jar).await?;
    let (connected, reconnect) = app
        .tenant(u.tenant_id.clone(), move |c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM google_tokens WHERE member_id = ?1",
                [&u.id],
                |r| r.get(0),
            )?;
            Ok((n > 0, google_tokens::needs_reconnect(c, &u.id)?))
        })
        .await?;
    Ok(Json(
        serde_json::json!({ "connected": connected, "needs_reconnect": reconnect }),
    ))
}

/// Revokes the grant at Google (best effort: a failure never keeps the token) and hard-deletes it.
pub(super) async fn disconnect(State(app): State<App>, jar: CookieJar) -> Res<StatusCode> {
    let u = app.current_user(&jar).await?;
    let k = google_tokens::SecretKey::from_hex(&app.cfg.secret_key).ok();
    let uid = u.id.clone();
    let token = app
        .tenant(u.tenant_id.clone(), move |c| {
            let t = k.and_then(|k| google_tokens::load(c, &k, &uid).ok().flatten());
            google_tokens::delete(c, &uid)?;
            Ok(t.map(|(t, _)| t))
        })
        .await?;
    if let Some(t) = token {
        revoke(&app, &u.id, &t).await;
    }
    Ok(StatusCode::NO_CONTENT)
}
