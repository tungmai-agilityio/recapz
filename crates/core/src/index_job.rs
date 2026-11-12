//! `index` job: chunk a meeting's transcript, embed each chunk and store it for Ask search.
use crate::{
    chunk::chunk,
    embed::{to_blob, Embedder},
    jobs::Job,
};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};

pub const KIND: &str = "index";

/// Safe to retry: the meeting's chunks (and their search rows, via triggers) are replaced.
pub fn handle(c: &Connection, job: &Job, embedder: &dyn Embedder) -> Result<()> {
    let mid = job
        .meeting_id
        .as_deref()
        .ok_or_else(|| anyhow!("job has no meeting"))?;
    let segments: Vec<(i64, i64, String)> = c
        .prepare(
            "SELECT start_ms, end_ms, text FROM segments WHERE meeting_id=?1 ORDER BY start_ms, id",
        )?
        .query_map([mid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;

    c.execute_batch("BEGIN")?;
    let stored = (|| -> Result<()> {
        c.execute("DELETE FROM chunks WHERE meeting_id=?1", [mid])?;
        for k in chunk(&segments) {
            c.execute(
                "INSERT INTO chunks (meeting_id, start_ms, end_ms, text, embedding) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![mid, k.start_ms, k.end_ms, k.text, to_blob(&embedder.embed(&k.text))],
            )?;
        }
        Ok(())
    })();
    c.execute_batch(if stored.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    stored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::open_tenant, embed::HashEmbedder, jobs};

    fn setup(texts: &[&str]) -> (tempfile::TempDir, Connection, Job) {
        let d = tempfile::tempdir().unwrap();
        let c = open_tenant(d.path(), "t").unwrap();
        c.execute(
            "INSERT INTO meetings (id, title, platform, status) VALUES ('m','T','upload','ready')",
            [],
        )
        .unwrap();
        for (i, t) in texts.iter().enumerate() {
            c.execute(
                "INSERT INTO segments (meeting_id, start_ms, end_ms, text) VALUES ('m', ?1, ?2, ?3)",
                params![i as i64 * 10_000, i as i64 * 10_000 + 10_000, t],
            ).unwrap();
        }
        let id = jobs::enqueue(&c, KIND, Some("m"), None).unwrap();
        let job = Job {
            id,
            kind: KIND.into(),
            meeting_id: Some("m".into()),
            payload: None,
        };
        (d, c, job)
    }

    fn count(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn stores_overlapping_chunks_and_search_rows_and_replaces_on_rerun() {
        let texts: Vec<String> = (0..12).map(|i| format!("word{i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let (_d, c, job) = setup(&refs);
        handle(&c, &job, &HashEmbedder).unwrap();
        handle(&c, &job, &HashEmbedder).unwrap();
        assert_eq!(count(&c, "SELECT COUNT(*) FROM chunks"), 3);
        assert_eq!(
            count(
                &c,
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'word11'"
            ),
            1
        );
        let blob: Vec<u8> = c
            .query_row("SELECT embedding FROM chunks LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(blob.len(), crate::embed::HASH_DIM * 4);
    }

    #[test]
    fn no_segments_means_no_chunks() {
        let (_d, c, job) = setup(&[]);
        handle(&c, &job, &HashEmbedder).unwrap();
        assert_eq!(count(&c, "SELECT COUNT(*) FROM chunks"), 0);
    }
}
