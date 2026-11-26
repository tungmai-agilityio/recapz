//! Workspaces of the signed-in person: list them and switch the session to another one.
use super::{ApiError, App, Res, SESSION_COOKIE};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use recapz_core::db;
use rusqlite::params;
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct WorkspaceOut {
    id: String,
    name: String,
    role: String,
    current: bool,
}

/// Accepts the pending invites for the person's email (a new workspace shows up without signing in
/// again), then lists their workspaces by name.
pub(super) async fn list(State(app): State<App>, jar: CookieJar) -> Res<Json<Vec<WorkspaceOut>>> {
    let u = app.current_user(&jar).await?;
    let out = app
        .control(move |c| {
            db::user_for_sign_in(c, &u.email, None)?;
            let mut st = c.prepare(
                "SELECT t.id, t.name, m.role FROM memberships m JOIN tenants t ON t.id = m.tenant_id
                 WHERE m.user_id = ?1 ORDER BY t.name, t.id",
            )?;
            let rows = st
                .query_map([&u.id], |r| {
                    let id: String = r.get(0)?;
                    Ok(WorkspaceOut {
                        current: id == u.tenant_id,
                        id,
                        name: r.get(1)?,
                        role: r.get(2)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(out))
}

/// Makes `id` the workspace of this session, and the one used at the next sign-in.
pub(super) async fn switch(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<StatusCode> {
    let u = app.current_user(&jar).await?;
    let sid = jar
        .get(SESSION_COOKIE)
        .ok_or(ApiError::Unauthorized)?
        .value()
        .to_owned();
    let member = app
        .control(move |c| {
            let tx = c.unchecked_transaction()?;
            let member: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM memberships WHERE user_id = ?1 AND tenant_id = ?2)",
                params![u.id, id],
                |r| r.get(0),
            )?;
            if member {
                tx.execute(
                    "UPDATE sessions SET tenant_id = ?1 WHERE id = ?2",
                    params![id, sid],
                )?;
                tx.execute(
                    "UPDATE users SET tenant_id = ?1 WHERE id = ?2",
                    params![id, u.id],
                )?;
                tx.commit()?;
            }
            Ok(member)
        })
        .await?;
    if !member {
        return Err(ApiError::Denied("you are not a member of this workspace"));
    }
    Ok(StatusCode::NO_CONTENT)
}
