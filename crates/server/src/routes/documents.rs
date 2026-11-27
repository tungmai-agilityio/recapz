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
    size_bytes: i64,
    /// Upload time, milliseconds since the epoch.
    created_at: i64,
}

#[derive(Serialize)]
pub struct Usage {
    used_bytes: i64,
    cap_bytes: i64,
}

/// What the member has stored against the per-member cap; only their own documents count.
pub async fn usage(State(app): State<App>, jar: CookieJar) -> Res<Json<Usage>> {
    let u = app.current_user(&jar).await?;
    let used_bytes = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            document::used_bytes(c, &u.id)
        })
        .await?;
    Ok(Json(Usage {
        used_bytes,
        cap_bytes: document::MAX_MEMBER_BYTES,
    }))
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

/// `indexing`, `indexed` or `no text found`: the one rule for the list and the document page.
fn status(c: &Connection, id: &str) -> anyhow::Result<&'static str> {
    let queued: i64 = c.query_row(
        "SELECT COUNT(*) FROM jobs WHERE kind=?1 AND payload=?2 AND state IN ('queued','running')",
        [document::KIND, id],
        |r| r.get(0),
    )?;
    let chunks: i64 = c.query_row(
        "SELECT COUNT(*) FROM document_chunks WHERE document_id=?1",
        [id],
        |r| r.get(0),
    )?;
    Ok(match (queued, chunks) {
        (q, _) if q > 0 => "indexing",
        (_, n) if n > 0 => "indexed",
        _ => "no text found",
    })
}

pub async fn list(State(app): State<App>, jar: CookieJar) -> Res<Json<Vec<DocRow>>> {
    let u = app.current_user(&jar).await?;
    let rows = app
        .tenant(u.tenant_id.clone(), move |c| {
            ensure_member(c, &u)?;
            document::list(c, &u.id)?
                .into_iter()
                .map(|d| {
                    let id = d.id.clone();
                    let status = status(c, &id)?;
                    Ok(DocRow {
                        id,
                        name: d.name,
                        llm_ok: d.llm_ok,
                        status,
                        size_bytes: d.size_bytes,
                        created_at: d.created_at,
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

/// How much of a file the page shows; the whole file stays available through `/file`.
const VIEW_BYTES: usize = 1024 * 1024;

#[derive(Serialize)]
pub struct Detail {
    id: String,
    name: String,
    /// `md` or `txt`.
    kind: String,
    size_bytes: i64,
    created_at: i64,
    llm_ok: bool,
    status: &'static str,
    /// The first 1 MB of the file, cut on a character boundary.
    text: String,
    /// True when the file is longer than what `text` holds.
    truncated: bool,
}

/// The member's own document and its stored file; `None` (a 404) for a missing or foreign id, an
/// unsafe id, or a file that is gone. `limit` bounds how many bytes are read.
async fn load(
    app: &App,
    jar: &CookieJar,
    id: String,
    limit: usize,
) -> Res<(document::Full, Vec<u8>, &'static str)> {
    let u = app.current_user(jar).await?;
    if !document::safe_id(&id) {
        return Err(ApiError::NotFound);
    }
    let (tenant, dir) = (u.tenant_id.clone(), app.cfg.data_dir.clone());
    app.tenant(u.tenant_id.clone(), move |c| {
        ensure_member(c, &u)?;
        let Some(d) = document::get(c, &u.id, &id)? else {
            return Ok(None);
        };
        let file =
            match std::fs::File::open(recapz_core::db::tenant_dir(&dir, &tenant).join(&d.path)) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e.into()),
            };
        let mut bytes = vec![];
        std::io::Read::read_to_end(&mut std::io::Read::take(file, limit as u64), &mut bytes)?;
        let status = status(c, &d.id)?;
        Ok(Some((d, bytes, status)))
    })
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn get(
    State(app): State<App>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Res<Json<Detail>> {
    let (d, mut bytes, status) = load(&app, &jar, id, VIEW_BYTES + 1).await?;
    let truncated = bytes.len() > VIEW_BYTES;
    bytes.truncate(VIEW_BYTES);
    // Uploads are valid UTF-8, so only a cut inside a character can fail: keep what is valid.
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t.to_owned(),
        Err(e) => String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(),
    };
    Ok(Json(Detail {
        id: d.id,
        name: d.name,
        kind: d.kind,
        size_bytes: d.size_bytes,
        created_at: d.created_at,
        llm_ok: d.llm_ok,
        status,
        text,
        truncated,
    }))
}

/// The original file as an attachment. `nosniff` and a fixed text type keep a browser from running
/// anything in it; `no-store` keeps it out of shared caches.
pub async fn file(State(app): State<App>, jar: CookieJar, Path(id): Path<String>) -> Res<Response> {
    let (d, bytes, _) = load(&app, &jar, id, document::MAX_FILE_BYTES).await?;
    tracing::info!(document = %d.id, "document downloaded");
    let kind = if d.kind == "md" {
        "text/markdown; charset=utf-8"
    } else {
        "text/plain; charset=utf-8"
    };
    Ok((
        [
            (header::CONTENT_TYPE, kind.to_owned()),
            (header::CONTENT_DISPOSITION, attachment_header(&d.name)),
            (
                header::HeaderName::from_static("x-content-type-options"),
                "nosniff".to_owned(),
            ),
            (header::CACHE_CONTROL, "private, no-store".to_owned()),
        ],
        bytes,
    )
        .into_response())
}
