//! Upcoming calendar meetings, the per-event join switch, and sending the notetaker to a pasted link.
use super::{ensure_member, ApiError, App, Created, Res};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use recapz_core::{calendar, db};
use recapz_notetaker::links;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

const UPCOMING_LIMIT: u32 = 50;
const MAX_TITLE: usize = 200;

#[derive(Serialize)]
pub(super) struct EventOut {
    id: String,
    title: String,
    starts_at: i64,
    ends_at: i64,
    platform: Option<String>,
    join_url: Option<String>,
    auto_join: bool,
    attendees: Vec<AttendeeOut>,
}

#[derive(Serialize)]
pub(super) struct AttendeeOut {
    name: Option<String>,
    email: String,
    status: Option<String>,
}

/// The signed-in member's own events, with the join link and invitees their calendar gave them.
pub(super) async fn upcoming(State(app): State<App>, jar: CookieJar) -> Res<Json<Vec<EventOut>>> {
    let u = app.current_user(&jar).await?;
    let uid = u.id.clone();
    let rows = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            calendar::upcoming(c, &uid, db::now_ms(), UPCOMING_LIMIT)
        })
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|e| EventOut {
                id: e.id,
                title: e.title,
                starts_at: e.starts_at,
                ends_at: e.ends_at,
                platform: e.platform,
                join_url: e.join_url,
                auto_join: e.auto_join,
                attendees: e
                    .attendees
                    .into_iter()
                    .map(|a| AttendeeOut {
                        name: a.name,
                        email: a.email,
                        status: a.status,
                    })
                    .collect(),
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub(super) struct Switch {
    auto_join: bool,
}

pub(super) async fn set_join(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(body): Json<Switch>,
) -> Res<StatusCode> {
    let u = app.current_user(&jar).await?;
    let uid = u.id.clone();
    let ok = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            calendar::set_auto_join(c, &uid, &id, body.auto_join)
        })
        .await?;
    if ok {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[derive(Deserialize)]
pub(super) struct JoinLink {
    url: String,
    title: Option<String>,
    /// `vi` (default), `en` or `multi`.
    language: Option<String>,
}

/// Queues a meeting for the notetaker to join: a `scheduled` row the notetaker driver picks up.
pub(super) async fn join_link(
    State(app): State<App>,
    jar: CookieJar,
    Json(body): Json<JoinLink>,
) -> Res<(StatusCode, Json<Created>)> {
    let u = app.current_user(&jar).await?;
    let link = links::parse(&body.url).ok_or(ApiError::BadRequest("not a Meet or Zoom link"))?;
    let language = super::meeting_language(body.language.as_deref())?;
    let title = body
        .title
        .map(|t| t.trim().chars().take(MAX_TITLE).collect::<String>())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            format!(
                "{} call",
                if link.platform() == "meet" {
                    "Meet"
                } else {
                    "Zoom"
                }
            )
        });
    let (new_id, platform, url) = (db::new_id(), link.platform(), link.url().to_string());
    // The same member sending the same link again (a double click) gets the meeting already queued,
    // so one call never gets two notetakers.
    let (id, created) = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            let existing: Option<String> = c
                .query_row(
                    "SELECT id FROM meetings WHERE owner_id = ?1 AND join_url = ?2
                       AND status IN ('scheduled','joining','recording') ORDER BY created_at LIMIT 1",
                    params![u.id, url],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = existing {
                return Ok((id, false));
            }
            c.execute(
                "INSERT INTO meetings (id, title, platform, join_url, status, owner_id, language, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'scheduled', ?5, ?6, ?7)",
                params![new_id, title, platform, url, u.id, language, db::now_ms()],
            )?;
            Ok((new_id, true))
        })
        .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(Created { id })))
}
