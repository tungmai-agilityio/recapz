//! A member's documents for Ask: upload, list, toggle "May be sent to the language model", delete.
//! Every query is scoped to the signed-in member; someone else's id answers 404, never 403. Logs
//! and errors carry ids only, never document text.
use super::*;
use recapz_core::{document, purge::purge_document};

/// Multipart framing allowance on top of the 20 MB file limit.
pub const BODY_LIMIT: usize = document::MAX_FILE_BYTES + 64 * 1024;

/// The only `document::add` failures that are the caller's fault; anything else is a server error.
const REJECTIONS: [&str; 4] = [
    "only Markdown and TXT files are supported",
    "file is over 20 MB",
    "file is not UTF-8 text",
    "document storage is full (500 MB)",
];

#[derive(Serialize)]
pub struct DocRow {
    id: String,
    name: String,
    llm_ok: bool,
    /// `indexing`, `indexed` or `no text found`.
    status: &'static str,
}

#[derive(Deserialize)]
pub struct Toggle {
    llm_ok: bool,
}

pub async fn upload(
    State(app): State<App>,
    jar: CookieJar,
    mut mp: Multipart,
) -> Res<(StatusCode, Json<Created>)> {
    let u = app.current_user(&jar).await?;
    let mut file = None;
    while let Some(f) = mp
        .next_field()
        .await
        .map_err(|_| ApiError::BadRequest("bad multipart body or file over 20 MB"))?
    {
        if f.name() == Some("file") {
            let name = f.file_name().unwrap_or("document").to_owned();
            let bytes = f
                .bytes()
                .await
                .map_err(|_| ApiError::BadRequest("bad multipart body or file over 20 MB"))?;
            file = Some((name, bytes));
        }
    }
    let (name, bytes) = file.ok_or(ApiError::BadRequest("missing file field"))?;
    let tenant = u.tenant_id.clone();
    let dir = app.cfg.data_dir.clone();
    let id = app
        .tenant(tenant.clone(), move |c| {
            ensure_member(c, &u)?;
            match document::add(c, &dir, &tenant, &u.id, &safe_name(&name), &bytes) {
                Ok(id) => Ok(Ok(id)),
                Err(e) => match REJECTIONS.iter().find(|m| **m == e.to_string()) {
                    Some(m) => Ok(Err(*m)),
                    None => Err(e),
                },
            }
        })
        .await?
        .map_err(ApiError::BadRequest)?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}

pub async fn list(State(app): State<App>, jar: CookieJar) -> Res<Json<Vec<DocRow>>> {
    let u = app.current_user(&jar).await?;
    let rows = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            document::list(c, &u.id)?
                .into_iter()
                .map(|(id, name, llm_ok)| {
                    let queued: i64 = c.query_row(
                        "SELECT COUNT(*) FROM jobs WHERE kind=?1 AND payload=?2 AND state IN ('queued','running')",
                        [document::KIND, &id],
                        |r| r.get(0),
                    )?;
                    let chunks: i64 = c.query_row(
                        "SELECT COUNT(*) FROM document_chunks WHERE document_id=?1",
                        [&id],
                        |r| r.get(0),
                    )?;
                    let status = match (queued, chunks) {
                        (q, _) if q > 0 => "indexing",
                        (_, n) if n > 0 => "indexed",
                        _ => "no text found",
                    };
                    Ok(DocRow {
                        id,
                        name,
                        llm_ok,
                        status,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .await?;
    Ok(Json(rows))
}

pub async fn set_llm_ok(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(body): Json<Toggle>,
) -> Res<StatusCode> {
    let u = app.current_user(&jar).await?;
    if !document::safe_id(&id) {
        return Err(ApiError::NotFound);
    }
    let found = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            document::set_llm_ok(c, &u.id, &id, body.llm_ok)
        })
        .await?;
    found
        .then_some(StatusCode::NO_CONTENT)
        .ok_or(ApiError::NotFound)
}

pub async fn delete(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<StatusCode> {
    let u = app.current_user(&jar).await?;
    if !document::safe_id(&id) {
        return Err(ApiError::NotFound);
    }
    let (tenant, dir) = (u.tenant_id.clone(), app.cfg.data_dir.clone());
    let found = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            purge_document(c, &dir, &tenant, &u.id, &id)
        })
        .await?;
    found
        .then_some(StatusCode::NO_CONTENT)
        .ok_or(ApiError::NotFound)
}
