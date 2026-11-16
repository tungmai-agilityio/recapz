//! Pulls each connected member's Google Calendar into their calendar rows. A refresh token is
//! exchanged for a short-lived access token that lives only in this function; `invalid_grant`
//! means Google no longer honours the grant, so the token is deleted and the member is asked to
//! reconnect. Events already synced stay. Tokens, keys and event text are never logged.
use crate::{config::Google, google_calendar};
use anyhow::{bail, Result};
use recapz_core::{
    calendar::{self, NewEvent},
    db, google_tokens,
    google_tokens::SecretKey,
    settings::{self, JoinRule},
};
use std::{path::Path, time::Duration};

const WINDOW_MS: i64 = 7 * 24 * 3600 * 1000;
/// Pages of 250 events read per member per sync, so a runaway reply cannot loop forever.
const MAX_PAGES: usize = 10;
const EVERY: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Synced(usize),
    /// Google refused the refresh token; it was deleted and the member must reconnect.
    Disconnected,
    NotConnected,
}

/// Reads the body in chunks and stops at `MAX_BODY_BYTES`, so a hostile reply cannot fill memory.
async fn body(mut res: reqwest::Response) -> Result<String> {
    let mut buf = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        buf.extend_from_slice(&chunk);
        if buf.len() > google_calendar::MAX_BODY_BYTES {
            bail!("calendar response too large");
        }
    }
    Ok(String::from_utf8(buf)?)
}

/// `Ok(None)` when Google answers `invalid_grant`.
async fn access_token(http: &reqwest::Client, g: &Google, refresh: &str) -> Result<Option<String>> {
    let res = http
        .post(&g.token_url)
        .form(&[
            ("refresh_token", refresh),
            ("client_id", &g.client_id),
            ("client_secret", &g.client_secret),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?;
    let status = res.status();
    let v: serde_json::Value = serde_json::from_str(&body(res).await?)?;
    if v.get("error").and_then(|e| e.as_str()) == Some("invalid_grant") {
        return Ok(None);
    }
    if !status.is_success() {
        bail!("token refresh failed: {status}");
    }
    match v.get("access_token").and_then(|t| t.as_str()) {
        Some(t) => Ok(Some(t.to_string())),
        None => bail!("token refresh returned no access token"),
    }
}

async fn fetch(
    http: &reqwest::Client,
    g: &Google,
    access: &str,
    from: i64,
    to: i64,
) -> Result<Vec<NewEvent>> {
    let mut all = Vec::new();
    let mut token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let res = http
            .get(&g.events_url)
            .bearer_auth(access)
            .query(&google_calendar::list_query(from, to, token.as_deref()))
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let reason = body(res)
                .await
                .ok()
                .and_then(|b| google_calendar::error_reason(&b));
            bail!(
                "calendar request failed: {status}{}",
                reason.map(|r| format!(" ({r})")).unwrap_or_default()
            );
        }
        let page = google_calendar::parse_page(&body(res).await?)?;
        all.extend(page.events);
        token = page.next_page_token;
        if token.is_none() {
            return Ok(all);
        }
    }
    // A partial list would make `calendar::sync` delete real events, so keep what is stored.
    bail!("too many calendar pages")
}

/// Syncs one member's next seven days. Any other failure (network, 5xx, bad reply) is an `Err`
/// and leaves the token and the stored events as they were.
pub async fn sync_member(
    data_dir: &Path,
    tenant: &str,
    member: &str,
    g: &Google,
    key: SecretKey,
    now: i64,
) -> Result<Outcome> {
    let (d, t, m) = (data_dir.to_owned(), tenant.to_owned(), member.to_owned());
    let token = tokio::task::spawn_blocking(move || -> Result<_> {
        google_tokens::load(&db::open_tenant(&d, &t)?, &key, &m)
    })
    .await??;
    let Some((refresh, _)) = token else {
        return Ok(Outcome::NotConnected);
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let (d, t, m) = (data_dir.to_owned(), tenant.to_owned(), member.to_owned());
    let Some(access) = access_token(&http, g, &refresh).await? else {
        tokio::task::spawn_blocking(move || -> Result<()> {
            google_tokens::mark_revoked(&db::open_tenant(&d, &t)?, &m)
        })
        .await??;
        return Ok(Outcome::Disconnected);
    };
    let to = now + WINDOW_MS;
    let events = fetch(&http, g, &access, now, to).await?;
    let n = events.len();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let c = db::open_tenant(&d, &t)?;
        let auto = settings::join_rule(&c)? == JoinRule::Auto;
        calendar::sync(&c, &m, &events, now, to, auto)
    })
    .await??;
    Ok(Outcome::Synced(n))
}

/// Every ten minutes, syncs every member who has a stored token. Needs a valid key and client.
pub fn spawn(data_dir: std::path::PathBuf, g: Google, key_hex: String) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(EVERY);
        loop {
            tick.tick().await;
            let d = data_dir.clone();
            let members = tokio::task::spawn_blocking(move || -> Result<Vec<(String, String)>> {
                let mut out = Vec::new();
                for t in db::tenant_ids(&db::open_control(&d)?)? {
                    let c = db::open_tenant(&d, &t)?;
                    let mut q = c.prepare("SELECT member_id FROM google_tokens")?;
                    for m in q.query_map([], |r| r.get::<_, String>(0))? {
                        out.push((t.clone(), m?));
                    }
                }
                Ok(out)
            })
            .await;
            let Ok(Ok(members)) = members else {
                tracing::error!("calendar sync: could not list members");
                continue;
            };
            for (tenant, member) in members {
                let Ok(key) = SecretKey::from_hex(&key_hex) else {
                    return;
                };
                match sync_member(&data_dir, &tenant, &member, &g, key, db::now_ms()).await {
                    Ok(Outcome::Disconnected) => {
                        tracing::warn!(member = %member, "Google calendar grant revoked; reconnect needed")
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(member = %member, "calendar sync failed: {e:#}"),
                }
            }
        }
    });
}
