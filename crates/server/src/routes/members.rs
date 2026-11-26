//! Members screen API (admin only): list, invite by link, change role, remove.
use super::{ApiError, App, Res};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use recapz_core::{db, members, members::Refused};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub(super) struct MemberOut {
    id: String,
    email: String,
    name: Option<String>,
    role: String,
    meetings: i64,
}

#[derive(Serialize)]
pub(super) struct InviteOut {
    email: String,
    role: String,
    created_at: i64,
}

#[derive(Serialize)]
pub(super) struct ListOut {
    members: Vec<MemberOut>,
    invites: Vec<InviteOut>,
}

#[derive(Deserialize)]
pub(super) struct NewInvite {
    email: String,
    role: String,
}

#[derive(Serialize)]
pub(super) struct Link {
    link: String,
}

#[derive(Deserialize)]
pub(super) struct NewRole {
    role: String,
}

async fn admin(app: &App, jar: &CookieJar) -> Res<db::User> {
    let u = app.current_user(jar).await?;
    if u.role != "admin" {
        return Err(ApiError::Denied("only an admin can manage members"));
    }
    Ok(u)
}

/// Turns a core refusal into its 4xx answer; anything else stays a 500.
fn refusal(e: ApiError) -> ApiError {
    let ApiError::Internal(e) = e else { return e };
    match e.downcast_ref::<Refused>() {
        Some(Refused::InvalidRole) => ApiError::BadRequest("role must be admin or member"),
        Some(Refused::AlreadyMember) => ApiError::Conflict("already a member of this workspace"),
        Some(Refused::NotMember) => ApiError::NotFound,
        Some(Refused::LastAdmin) => ApiError::Conflict("a workspace needs at least one admin"),
        Some(Refused::SelfRemoval) => ApiError::Denied("you cannot remove yourself"),
        None => ApiError::Internal(e),
    }
}

pub(super) async fn list(State(app): State<App>, jar: CookieJar) -> Res<Json<ListOut>> {
    let u = admin(&app, &jar).await?;
    let dir = app.cfg.data_dir.clone();
    let (m, i) = app
        .control(move |c| {
            let tenant = db::open_tenant(&dir, &u.tenant_id)?;
            members::list(c, &tenant, &u.tenant_id)
        })
        .await?;
    Ok(Json(ListOut {
        members: m
            .into_iter()
            .map(|m| MemberOut {
                id: m.id,
                email: m.email,
                name: m.name,
                role: m.role,
                meetings: m.meetings,
            })
            .collect(),
        invites: i
            .into_iter()
            .map(|i| InviteOut {
                email: i.email,
                role: i.role,
                created_at: i.created_at,
            })
            .collect(),
    }))
}

pub(super) async fn invite(
    State(app): State<App>,
    jar: CookieJar,
    Json(b): Json<NewInvite>,
) -> Res<(StatusCode, Json<Link>)> {
    let u = admin(&app, &jar).await?;
    if !b.email.contains('@') {
        return Err(ApiError::BadRequest("enter a valid email"));
    }
    app.control(move |c| members::invite(c, &u.tenant_id, &u.id, &b.email, &b.role))
        .await
        .map_err(refusal)?;
    let link = format!("{}/", app.cfg.base_url.trim_end_matches('/'));
    Ok((StatusCode::CREATED, Json(Link { link })))
}

pub(super) async fn cancel_invite(
    State(app): State<App>,
    jar: CookieJar,
    Path(email): Path<String>,
) -> Res<StatusCode> {
    let u = admin(&app, &jar).await?;
    if app
        .control(move |c| members::cancel_invite(c, &u.tenant_id, &email))
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub(super) async fn set_role(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(b): Json<NewRole>,
) -> Res<StatusCode> {
    let u = admin(&app, &jar).await?;
    app.control(move |c| members::set_role(c, &u.tenant_id, &id, &b.role))
        .await
        .map_err(refusal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn remove(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<StatusCode> {
    let u = admin(&app, &jar).await?;
    let dir = app.cfg.data_dir.clone();
    app.control(move |c| {
        let tenant = db::open_tenant(&dir, &u.tenant_id)?;
        members::remove(c, &tenant, &dir, &u.tenant_id, &u.id, &id)
    })
    .await
    .map_err(refusal)?;
    Ok(StatusCode::NO_CONTENT)
}
