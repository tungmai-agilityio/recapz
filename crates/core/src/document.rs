//! Member documents (Markdown and TXT) for Ask: store the file, chunk it, and embed it only when the
//! uploader allowed "May be sent to the language model". Documents belong to the uploader alone.
use crate::{
    db::{new_id, now_ms, tenant_dir},
    embed::{to_blob, Embedder},
    jobs::{self, Job},
};
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::{fs, path::Path};

pub const KIND: &str = "ingest_document";
pub const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_MEMBER_BYTES: i64 = 500 * 1024 * 1024;
const CHUNK_CHARS: usize = 1200;

/// A document id is a UUID; anything else must never reach a file path.
pub fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Stores the file under `documents/` (never `audio/`), with the language-model toggle off, and
/// queues ingest. Returns the document id.
pub fn add(
    c: &Connection,
    data_dir: &Path,
    tenant_id: &str,
    member_id: &str,
    filename: &str,
    bytes: &[u8],
) -> Result<String> {
    let ext = filename.rsplit_once('.').map(|(_, e)| e.to_lowercase());
    let kind = match ext.as_deref() {
        Some("md" | "markdown") => "md",
        Some("txt") => "txt",
        _ => bail!("only Markdown and TXT files are supported"),
    };
    if bytes.len() > MAX_FILE_BYTES {
        bail!("file is over 20 MB");
    }
    if std::str::from_utf8(bytes).is_err() {
        bail!("file is not UTF-8 text");
    }
    if used_bytes(c, member_id)? + bytes.len() as i64 > MAX_MEMBER_BYTES {
        bail!("document storage is full (500 MB)");
    }
    let id = new_id();
    let rel = format!("documents/{id}.{kind}");
    let abs = tenant_dir(data_dir, tenant_id).join(&rel);
    fs::create_dir_all(abs.parent().unwrap())?;
    fs::write(&abs, bytes)?;
    let stored = c
        .execute(
            "INSERT INTO documents (id, member_id, name, kind, size_bytes, path, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![id, member_id, filename, kind, bytes.len() as i64, rel, now_ms()],
        )
        .map_err(anyhow::Error::from)
        .and_then(|_| jobs::enqueue(c, KIND, None, Some(&id)));
    if let Err(e) = stored {
        let _ = fs::remove_file(&abs);
        let _ = c.execute("DELETE FROM documents WHERE id=?1", [&id]);
        return Err(e);
    }
    Ok(id)
}

/// Turns "May be sent to the language model" on or off. Off drops the document's vectors at once;
/// on queues ingest to embed it. False when the member does not own the document.
pub fn set_llm_ok(c: &Connection, member_id: &str, doc_id: &str, on: bool) -> Result<bool> {
    c.execute_batch("BEGIN")?;
    let done = (|| -> Result<bool> {
        let n = c.execute(
            "UPDATE documents SET llm_ok=?3 WHERE id=?2 AND member_id=?1",
            params![member_id, doc_id, on as i64],
        )?;
        if n == 0 {
            return Ok(false);
        }
        if on {
            jobs::enqueue(c, KIND, None, Some(doc_id))?;
        } else {
            c.execute(
                "DELETE FROM document_vectors WHERE document_id=?1",
                [doc_id],
            )?;
        }
        Ok(true)
    })();
    c.execute_batch(if done.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    done
}

/// Bytes of the member's own documents, counted against `MAX_MEMBER_BYTES`.
pub fn used_bytes(c: &Connection, member_id: &str) -> Result<i64> {
    Ok(c.query_row(
        "SELECT COALESCE(SUM(size_bytes),0) FROM documents WHERE member_id=?1",
        [member_id],
        |r| r.get(0),
    )?)
}

/// One of the member's documents with where its file is (relative to the workspace folder).
#[derive(Debug, PartialEq)]
pub struct Full {
    pub id: String,
    pub name: String,
    /// `md` or `txt`.
    pub kind: String,
    pub llm_ok: bool,
    pub size_bytes: i64,
    pub created_at: i64,
    pub path: String,
}

/// The member's own document, or `None` for a missing one or someone else's.
pub fn get(c: &Connection, member_id: &str, doc_id: &str) -> Result<Option<Full>> {
    Ok(c.query_row(
        "SELECT id, name, kind, llm_ok, size_bytes, created_at, path FROM documents WHERE id=?1 AND member_id=?2",
        [doc_id, member_id],
        |r| {
            Ok(Full {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                llm_ok: r.get::<_, i64>(3)? == 1,
                size_bytes: r.get(4)?,
                created_at: r.get(5)?,
                path: r.get(6)?,
            })
        },
    )
    .optional()?)
}

/// One of the member's documents as listed.
#[derive(Debug, PartialEq)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub llm_ok: bool,
    pub size_bytes: i64,
    /// Upload time, milliseconds since the epoch.
    pub created_at: i64,
}

/// The member's documents, newest first. Only the member's own.
pub fn list(c: &Connection, member_id: &str) -> Result<Vec<Listed>> {
    Ok(c.prepare(
        "SELECT id, name, llm_ok, size_bytes, created_at FROM documents WHERE member_id=?1 ORDER BY created_at DESC, id DESC",
    )?
    .query_map([member_id], |r| {
        Ok(Listed {
            id: r.get(0)?,
            name: r.get(1)?,
            llm_ok: r.get::<_, i64>(2)? == 1,
            size_bytes: r.get(3)?,
            created_at: r.get(4)?,
        })
    })?
    .collect::<Result<_, _>>()?)
}

/// Paragraph-packed pieces of at most `CHUNK_CHARS` characters (a longer paragraph is cut).
fn split(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut cur = String::new();
    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        let chars: Vec<char> = para.chars().collect();
        for piece in chars.chunks(CHUNK_CHARS) {
            if !cur.is_empty() && cur.chars().count() + piece.len() + 2 > CHUNK_CHARS {
                out.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push_str("\n\n");
            }
            cur.extend(piece);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `ingest_document` job (document id in the payload). Chunks the file and fills the full-text
/// index; embeds only when the document's toggle is on. The embedder is never called, so no text
/// leaves the server, for a document with the toggle off. Safe to retry: chunks are replaced.
pub fn handle(
    c: &Connection,
    data_dir: &Path,
    tenant_id: &str,
    job: &Job,
    embedder: &dyn Embedder,
) -> Result<()> {
    let id = job
        .payload
        .as_deref()
        .ok_or_else(|| anyhow!("job has no document"))?;
    if !safe_id(id) {
        bail!("unsafe document id");
    }
    let Some((path, llm_ok)) = c
        .query_row(
            "SELECT path, llm_ok FROM documents WHERE id=?1",
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? == 1)),
        )
        .optional()?
    else {
        return Ok(()); // deleted while queued
    };
    let text = fs::read_to_string(tenant_dir(data_dir, tenant_id).join(path))?;
    let pieces = split(&text);
    // Embed before taking the write lock; a failure leaves the old chunks as they were.
    let blobs = if llm_ok {
        pieces
            .iter()
            .map(|p| embedder.embed(p).map(|v| to_blob(&v)))
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![]
    };
    c.execute_batch("BEGIN")?;
    let stored = (|| -> Result<()> {
        // The toggle may have flipped, or the document been deleted, while embedding: re-read it
        // and store vectors only if it is still on.
        let Some(on_now) = c
            .query_row("SELECT llm_ok FROM documents WHERE id=?1", [id], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
        else {
            return Ok(());
        };
        c.execute("DELETE FROM document_chunks WHERE document_id=?1", [id])?;
        for (i, p) in pieces.iter().enumerate() {
            c.execute(
                "INSERT INTO document_chunks (document_id, seq, text) VALUES (?1,?2,?3)",
                params![id, i as i64, p],
            )?;
            let chunk_id = c.last_insert_rowid();
            if let (1, Some(b)) = (on_now, blobs.get(i)) {
                c.execute(
                    "INSERT INTO document_vectors (chunk_id, document_id, embedder, embedding) VALUES (?1,?2,?3,?4)",
                    params![chunk_id, id, embedder.id(), b],
                )?;
            }
        }
        Ok(())
    })();
    c.execute_batch(if stored.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    stored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::open_tenant, embed::HashEmbedder, purge::purge_document};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    /// Counts calls; stands in for a hosted embedder.
    #[derive(Default)]
    struct Counting {
        calls: AtomicUsize,
        // Runs once on the first call, to flip the toggle mid-embed.
        during: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }
    impl Embedder for Counting {
        fn id(&self) -> &'static str {
            "hosted-test"
        }
        fn embed(&self, text: &str) -> Result<Vec<f32>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(f) = self.during.lock().unwrap().take() {
                f();
            }
            HashEmbedder.embed(text)
        }
    }

    fn count(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn setup() -> (tempfile::TempDir, Connection) {
        let d = tempfile::tempdir().unwrap();
        let c = open_tenant(d.path(), "t").unwrap();
        c.execute_batch(
            "INSERT INTO members (id,email,role) VALUES ('m1','a@x','member'),('m2','b@x','member')",
        )
        .unwrap();
        (d, c)
    }

    fn job(c: &Connection, id: &str) -> Job {
        let (job_id, kind, payload): (i64, String, Option<String>) = c
            .query_row(
                "SELECT id, kind, payload FROM jobs WHERE payload=?1 ORDER BY id DESC",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        Job {
            id: job_id,
            kind,
            meeting_id: None,
            payload,
        }
    }

    const BODY: &[u8] = b"Zebrafish budget.\n\nWalrus plan for Q3.";

    #[test]
    fn toggle_off_never_calls_the_embedder_and_is_keyword_only() {
        let (d, c) = setup();
        let id = add(&c, d.path(), "t", "m1", "n.md", BODY).unwrap();
        assert_eq!(count(&c, "SELECT llm_ok FROM documents"), 0, "default off");
        let e = Counting::default();
        handle(&c, d.path(), "t", &job(&c, &id), &e).unwrap();
        assert_eq!(
            e.calls.load(Ordering::SeqCst),
            0,
            "no text sent to a provider"
        );
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_chunks"), 1);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_vectors"), 0);
        assert_eq!(
            count(&c, "SELECT COUNT(*) FROM document_chunks_fts WHERE document_chunks_fts MATCH 'zebrafish'"),
            1
        );
        // Re-running ingest (retry) still never embeds.
        handle(&c, d.path(), "t", &job(&c, &id), &e).unwrap();
        assert_eq!(e.calls.load(Ordering::SeqCst), 0);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_chunks"), 1);
    }

    #[test]
    fn flipping_on_embeds_and_flipping_off_drops_vectors_but_keeps_keyword_search() {
        let (d, c) = setup();
        let id = add(&c, d.path(), "t", "m1", "n.txt", BODY).unwrap();
        let e = Counting::default();
        handle(&c, d.path(), "t", &job(&c, &id), &e).unwrap();
        assert!(set_llm_ok(&c, "m1", &id, true).unwrap());
        handle(&c, d.path(), "t", &job(&c, &id), &e).unwrap();
        assert_eq!(e.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            count(
                &c,
                "SELECT COUNT(*) FROM document_vectors WHERE embedder='hosted-test'"
            ),
            1
        );
        assert!(set_llm_ok(&c, "m1", &id, false).unwrap());
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_vectors"), 0);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_chunks"), 1);
        // A stale queued ingest after flipping off must not embed again.
        handle(&c, d.path(), "t", &job(&c, &id), &e).unwrap();
        assert_eq!(e.calls.load(Ordering::SeqCst), 1);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_vectors"), 0);
    }

    #[test]
    fn flipping_off_while_embedding_stores_no_vectors() {
        let (d, c) = setup();
        let id = add(&c, d.path(), "t", "m1", "n.md", BODY).unwrap();
        set_llm_ok(&c, "m1", &id, true).unwrap();
        let other = Mutex::new(open_tenant(d.path(), "t").unwrap());
        let e = Counting::default();
        let id2 = id.clone();
        *e.during.lock().unwrap() = Some(Box::new(move || {
            set_llm_ok(&other.lock().unwrap(), "m1", &id2, false).unwrap();
        }));
        handle(&c, d.path(), "t", &job(&c, &id), &e).unwrap();
        assert_eq!(e.calls.load(Ordering::SeqCst), 1);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_vectors"), 0);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_chunks"), 1);
    }

    #[test]
    fn only_the_uploader_can_list_or_toggle() {
        let (d, c) = setup();
        let id = add(&c, d.path(), "t", "m1", "n.md", BODY).unwrap();
        assert!(!set_llm_ok(&c, "m2", &id, true).unwrap());
        assert_eq!(count(&c, "SELECT llm_ok FROM documents"), 0);
        assert!(list(&c, "m2").unwrap().is_empty());
        let mine = list(&c, "m1").unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(
            (mine[0].id.as_str(), mine[0].name.as_str(), mine[0].llm_ok),
            (id.as_str(), "n.md", false)
        );
        assert_eq!(mine[0].size_bytes, BODY.len() as i64);
        assert_eq!(used_bytes(&c, "m1").unwrap(), BODY.len() as i64);
        assert_eq!(used_bytes(&c, "m2").unwrap(), 0);
    }

    #[test]
    fn rejects_pdf_binary_and_oversize_and_stores_outside_audio() {
        let (d, c) = setup();
        assert!(add(&c, d.path(), "t", "m1", "a.pdf", b"x").is_err());
        assert!(add(&c, d.path(), "t", "m1", "a.md", &[0xff, 0xfe]).is_err());
        assert!(add(
            &c,
            d.path(),
            "t",
            "m1",
            "a.txt",
            &vec![b'a'; MAX_FILE_BYTES + 1]
        )
        .is_err());
        assert_eq!(count(&c, "SELECT COUNT(*) FROM documents"), 0);
        let id = add(&c, d.path(), "t", "m1", "a.md", b"hi").unwrap();
        let t = tenant_dir(d.path(), "t");
        assert!(t.join(format!("documents/{id}.md")).exists());
        assert!(!t.join("audio").exists());
    }

    #[test]
    fn split_packs_paragraphs_and_cuts_long_ones() {
        assert_eq!(split("a\n\nb\n\n\n\nc"), vec!["a\n\nb\n\nc"]);
        let long = "x".repeat(CHUNK_CHARS + 5);
        let p = split(&format!("{long}\n\nend"));
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].len(), CHUNK_CHARS);
        assert_eq!(p[1], format!("{}\n\nend", "x".repeat(5)));
        assert!(split("  \n\n ").is_empty());
    }

    #[test]
    fn purge_removes_file_chunks_search_rows_vectors_and_job_for_that_document_only() {
        let (d, c) = setup();
        let a = add(&c, d.path(), "t", "m1", "a.md", b"Zebrafish one").unwrap();
        let b = add(&c, d.path(), "t", "m1", "b.md", b"Walrus two").unwrap();
        for id in [&a, &b] {
            set_llm_ok(&c, "m1", id, true).unwrap();
            handle(&c, d.path(), "t", &job(&c, id), &HashEmbedder).unwrap();
        }
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_vectors"), 2);
        assert!(
            !purge_document(&c, d.path(), "t", "m2", &a).unwrap(),
            "not the owner"
        );
        assert_eq!(count(&c, "SELECT COUNT(*) FROM documents"), 2);

        assert!(purge_document(&c, d.path(), "t", "m1", &a).unwrap());
        // Document b keeps its row, one chunk, one vector and its two ingest jobs (add, toggle on).
        for (table, col, kept) in [
            ("documents", "id", 1),
            ("document_chunks", "document_id", 1),
            ("document_vectors", "document_id", 1),
            ("jobs", "payload", 2),
        ] {
            assert_eq!(
                count(
                    &c,
                    &format!("SELECT COUNT(*) FROM {table} WHERE {col}='{a}'")
                ),
                0,
                "{table}"
            );
            assert_eq!(
                count(
                    &c,
                    &format!("SELECT COUNT(*) FROM {table} WHERE {col}='{b}'")
                ),
                kept,
                "{table}: other stays"
            );
        }
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_chunks_fts WHERE document_chunks_fts MATCH 'zebrafish'"), 0);
        assert_eq!(
            count(
                &c,
                "SELECT COUNT(*) FROM document_chunks_fts WHERE document_chunks_fts MATCH 'walrus'"
            ),
            1
        );
        let t = tenant_dir(d.path(), "t");
        assert!(!t.join(format!("documents/{a}.md")).exists());
        assert!(t.join(format!("documents/{b}.md")).exists());
        assert!(
            !purge_document(&c, d.path(), "t", "m1", &a).unwrap(),
            "second call finds nothing"
        );
        // A queued ingest for a deleted document is a no-op.
        handle(
            &c,
            d.path(),
            "t",
            &Job {
                id: 0,
                kind: KIND.into(),
                meeting_id: None,
                payload: Some(a),
            },
            &HashEmbedder,
        )
        .unwrap();
        assert_eq!(count(&c, "SELECT COUNT(*) FROM document_chunks"), 1);
    }
}
