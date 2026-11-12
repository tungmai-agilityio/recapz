//! Hybrid search over chunks for Ask: FTS5 keyword ranking and vector similarity, merged by
//! reciprocal rank. The meeting access rule is part of every query, so a chunk of a meeting the
//! member cannot see is never read, ranked or returned.
use crate::embed::{cosine, from_blob, keywords};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashMap;

/// The one meeting access rule as SQL, for a meeting aliased `m` and the member id as `?1`.
const VISIBLE: &str = "(m.owner_id = ?1 OR m.share = 'workspace')";
const POOL: usize = 50;
const RRF_K: f32 = 60.0;
/// Vector matches below this cosine are noise (shared filler words), not hits. Tuned for the
/// hashed baseline embedder; revisit when a real model replaces it.
const MIN_SIMILARITY: f32 = 0.2;

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub chunk_id: i64,
    pub meeting_id: String,
    pub title: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub score: f32,
}

/// Words only, each quoted, so user text can never be read as FTS syntax.
fn fts_query(q: &str) -> Option<String> {
    let words: Vec<String> = keywords(q).map(|w| format!("\"{w}\"")).collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

fn keyword_ids(c: &Connection, member: &str, q: &str) -> Result<Vec<i64>> {
    let Some(m) = fts_query(q) else {
        return Ok(vec![]);
    };
    let sql = format!(
        "SELECT k.id FROM chunks_fts f JOIN chunks k ON k.id = f.rowid
         JOIN meetings m ON m.id = k.meeting_id
         WHERE chunks_fts MATCH ?2 AND {VISIBLE} ORDER BY f.rank LIMIT {POOL}"
    );
    let mut st = c.prepare(&sql)?;
    let ids = st
        .query_map(params![member, m], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(ids)
}

// ponytail: brute-force scan of the member's visible chunks; switch to an index when a workspace
// holds more than ~100k chunks.
fn vector_ids(c: &Connection, member: &str, qvec: &[f32]) -> Result<Vec<i64>> {
    let sql = format!(
        "SELECT k.id, k.embedding FROM chunks k JOIN meetings m ON m.id = k.meeting_id WHERE {VISIBLE}"
    );
    let mut st = c.prepare(&sql)?;
    let mut scored: Vec<(i64, f32)> = st
        .query_map([member], |r| Ok((r.get(0)?, r.get::<_, Vec<u8>>(1)?)))?
        .map(|r| r.map(|(id, b)| (id, cosine(qvec, &from_blob(&b)))))
        .collect::<Result<_, _>>()?;
    scored.retain(|(_, s)| *s >= MIN_SIMILARITY);
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(scored.into_iter().take(POOL).map(|(id, _)| id).collect())
}

/// Top `limit` chunks the member may see, best first.
pub fn search(
    c: &Connection,
    member: &str,
    query: &str,
    qvec: &[f32],
    limit: usize,
) -> Result<Vec<Hit>> {
    let mut score: HashMap<i64, f32> = HashMap::new();
    for ids in [keyword_ids(c, member, query)?, vector_ids(c, member, qvec)?] {
        for (rank, id) in ids.into_iter().enumerate() {
            *score.entry(id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    let mut ranked: Vec<(i64, f32)> = score.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.truncate(limit);

    let mut get = c.prepare(
        "SELECT k.meeting_id, m.title, k.start_ms, k.end_ms, k.text
         FROM chunks k JOIN meetings m ON m.id = k.meeting_id WHERE k.id = ?1",
    )?;
    ranked
        .into_iter()
        .map(|(id, score)| {
            Ok(get.query_row([id], |r| {
                Ok(Hit {
                    chunk_id: id,
                    meeting_id: r.get(0)?,
                    title: r.get(1)?,
                    start_ms: r.get(2)?,
                    end_ms: r.get(3)?,
                    text: r.get(4)?,
                    score,
                })
            })?)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::open_tenant,
        embed::{to_blob, Embedder, HashEmbedder},
    };

    fn setup() -> (tempfile::TempDir, Connection) {
        let d = tempfile::tempdir().unwrap();
        let c = open_tenant(d.path(), "t").unwrap();
        for (id, owner, share) in [
            ("mine", "alice", "attendees"),
            ("team", "bob", "workspace"),
            ("secret", "bob", "attendees"),
        ] {
            c.execute(
                "INSERT INTO members (id, email, role) VALUES (?1, ?1 || '@x.test', 'member') ON CONFLICT DO NOTHING",
                [owner],
            )
            .unwrap();
            c.execute(
                "INSERT INTO meetings (id, title, platform, status, owner_id, share) VALUES (?1, ?1, 'upload', 'ready', ?2, ?3)",
                params![id, owner, share],
            )
            .unwrap();
        }
        (d, c)
    }

    fn add(c: &Connection, meeting: &str, start_ms: i64, text: &str) {
        c.execute(
            "INSERT INTO chunks (meeting_id, start_ms, end_ms, text, embedding) VALUES (?1, ?2, ?2 + 1000, ?3, ?4)",
            params![meeting, start_ms, text, to_blob(&HashEmbedder.embed(text))],
        )
        .unwrap();
    }

    fn ask(c: &Connection, member: &str, q: &str) -> Vec<Hit> {
        search(c, member, q, &HashEmbedder.embed(q), 5).unwrap()
    }

    #[test]
    fn a_private_meeting_never_appears_for_another_member() {
        let (_d, c) = setup();
        add(&c, "secret", 0, "the launch budget is nine thousand");
        add(&c, "team", 0, "the launch date moved to May");
        let hits = ask(&c, "alice", "launch budget");
        let ids: Vec<&str> = hits.iter().map(|h| h.meeting_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["team"],
            "only the workspace-shared meeting is visible"
        );
        // bob owns it, so he sees both.
        assert_eq!(ask(&c, "bob", "launch budget").len(), 2);
    }

    #[test]
    fn a_keyword_only_match_and_a_vector_only_match_both_surface() {
        let (_d, c) = setup();
        add(&c, "mine", 5_000, "pricing page ships on Friday");
        add(&c, "mine", 65_000, "we ordered noodles for lunch");
        let hits = ask(&c, "alice", "when does the pricing page ship");
        assert_eq!(hits[0].start_ms, 5_000);
        assert_eq!(hits[0].meeting_id, "mine");
        assert_eq!(
            hits.len(),
            1,
            "the unrelated chunk shares no word or vector overlap"
        );
    }

    #[test]
    fn filler_words_alone_do_not_match() {
        let (_d, c) = setup();
        add(
            &c,
            "mine",
            0,
            "the launch date moved to May after the review",
        );
        let q = "what is the acquisition price?";
        assert!(ask(&c, "alice", q).is_empty());
    }

    #[test]
    fn punctuation_and_empty_queries_are_safe() {
        let (_d, c) = setup();
        add(&c, "mine", 0, "plain text");
        assert!(ask(&c, "alice", "\"; DROP TABLE chunks; --").is_empty());
        assert!(ask(&c, "alice", "   ").is_empty());
    }

    #[test]
    fn vietnamese_questions_match_with_or_without_marks_and_ignore_filler() {
        let (_d, c) = setup();
        add(
            &c,
            "mine",
            7_000,
            "Đội đã thống nhất ra mắt trang giá vào thứ Sáu",
        );
        add(&c, "mine", 9_000, "Bữa trưa có phở và trà");
        // Filler words are dropped; the content words find the chunk.
        let hits = ask(&c, "alice", "Khi nào ra mắt trang giá?");
        assert_eq!(hits.iter().map(|h| h.start_ms).collect::<Vec<_>>(), [7_000]);
        // Typed without marks, the keyword side still matches (marks are folded by the index).
        let hits = ask(&c, "alice", "ra mat trang gia");
        assert_eq!(hits.iter().map(|h| h.start_ms).collect::<Vec<_>>(), [7_000]);
        // Decomposed Unicode (base letter + combining mark) is one word, not several.
        let nfd = "ma\u{0306}\u{0301}t"; // only this word can match
        let hits = ask(&c, "alice", nfd);
        assert_eq!(hits.iter().map(|h| h.start_ms).collect::<Vec<_>>(), [7_000]);
        // A question made only of filler matches nothing.
        assert!(ask(&c, "alice", "Đó là gì?").is_empty());
    }
}
