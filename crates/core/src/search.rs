//! Hybrid search over chunks for Ask: FTS5 keyword ranking and vector similarity, merged by
//! reciprocal rank. The meeting access rule is part of every query, so a chunk of a meeting the
//! member cannot see is never read, ranked or returned. The member's own documents are searched
//! with a separate rule (`documents.member_id`): the uploader only, never the meeting rule.
use crate::embed::{cosine, from_blob, keywords, Embedder};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashMap;

/// The one meeting access rule as SQL, for a meeting aliased `m` and the member id as `?1`.
use crate::db::MEETING_VISIBLE as VISIBLE;
/// The document access rule as SQL, for a document aliased `d` and the member id as `?1`.
const OWN_DOC: &str = "d.member_id = ?1";
/// A chunk is a meeting chunk (`false`) or a document chunk (`true`); their ids are separate.
type Key = (bool, i64);
const POOL: usize = 50;
const RRF_K: f32 = 60.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub chunk_id: i64,
    /// Exactly one of `meeting_id` and `document_id` is set.
    pub meeting_id: Option<String>,
    pub document_id: Option<String>,
    /// The meeting title or the document name.
    pub title: String,
    /// Moment in the meeting; 0 for a document.
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub score: f32,
    /// False for a document whose owner did not allow sending it to the language model: show it as
    /// a cited passage, never build a generated answer from its text.
    pub llm_ok: bool,
}

/// Words only, each quoted, so user text can never be read as FTS syntax.
fn fts_query(q: &str) -> Option<String> {
    let words: Vec<String> = keywords(q).map(|w| format!("\"{w}\"")).collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

fn keyword_ids(c: &Connection, member: &str, q: &str) -> Result<Vec<Key>> {
    let Some(m) = fts_query(q) else {
        return Ok(vec![]);
    };
    let mut out = vec![];
    for (doc, sql) in [
        (
            false,
            format!(
                "SELECT k.id, f.rank FROM chunks_fts f JOIN chunks k ON k.id = f.rowid
                 JOIN meetings m ON m.id = k.meeting_id
                 WHERE chunks_fts MATCH ?2 AND {VISIBLE} ORDER BY f.rank LIMIT {POOL}"
            ),
        ),
        (
            true,
            format!(
                "SELECT k.id, f.rank FROM document_chunks_fts f JOIN document_chunks k ON k.id = f.rowid
                 JOIN documents d ON d.id = k.document_id
                 WHERE document_chunks_fts MATCH ?2 AND {OWN_DOC} ORDER BY f.rank LIMIT {POOL}"
            ),
        ),
    ] {
        let mut st = c.prepare(&sql)?;
        let rows: Vec<(i64, f64)> = st
            .query_map(params![member, m], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        out.extend(rows.into_iter().map(|(id, r)| ((doc, id), r)));
    }
    // bm25 ranks (lower is better) from the two indexes are merged as if comparable.
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out.truncate(POOL);
    Ok(out.into_iter().map(|(k, _)| k).collect())
}

// ponytail: brute-force scan of the member's visible chunks; switch to an index when a workspace
// holds more than ~100k chunks.
fn vector_ids(
    c: &Connection,
    member: &str,
    e: &dyn Embedder,
    query: &str,
    min_similarity: f32,
) -> Result<Vec<Key>> {
    let qvec = e.embed(query)?;
    // Only chunks embedded by this same embedder are comparable; the rest are reached by keyword
    // until they are re-indexed. A document vector is read only while its toggle is on.
    let mut scored: Vec<(Key, f32)> = vec![];
    for (doc, sql) in [
        (
            false,
            format!(
                "SELECT k.id, k.embedding FROM chunks k JOIN meetings m ON m.id = k.meeting_id
                 WHERE {VISIBLE} AND k.embedder = ?2"
            ),
        ),
        (
            true,
            format!(
                "SELECT v.chunk_id, v.embedding FROM document_vectors v JOIN documents d ON d.id = v.document_id
                 WHERE {OWN_DOC} AND d.llm_ok = 1 AND v.embedder = ?2"
            ),
        ),
    ] {
        let mut st = c.prepare(&sql)?;
        let rows = st
            .query_map(params![member, e.id()], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        scored.extend(
            rows.into_iter()
                .map(|(id, b)| ((doc, id), cosine(&qvec, &from_blob(&b)))),
        );
    }
    scored.retain(|(_, s)| *s >= min_similarity);
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(scored.into_iter().take(POOL).map(|(k, _)| k).collect())
}

/// Top `limit` chunks the member may see, best first. `embedder` embeds the query and must be the
/// one that indexed the chunks for the vector side to see them; its `min_similarity` is the cut-off.
pub fn search(
    c: &Connection,
    member: &str,
    query: &str,
    embedder: &dyn Embedder,
    limit: usize,
) -> Result<Vec<Hit>> {
    search_at(c, member, query, embedder, limit, embedder.min_similarity())
}

/// `search` with an explicit cosine cut-off, for measuring thresholds (`eval ask`).
pub fn search_at(
    c: &Connection,
    member: &str,
    query: &str,
    embedder: &dyn Embedder,
    limit: usize,
    min_similarity: f32,
) -> Result<Vec<Hit>> {
    let mut score: HashMap<Key, f32> = HashMap::new();
    for ids in [
        keyword_ids(c, member, query)?,
        vector_ids(c, member, embedder, query, min_similarity)?,
    ] {
        for (rank, id) in ids.into_iter().enumerate() {
            *score.entry(id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    let mut ranked: Vec<(Key, f32)> = score.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.truncate(limit);

    let mut get_meeting = c.prepare(
        "SELECT k.meeting_id, m.title, k.start_ms, k.end_ms, k.text
         FROM chunks k JOIN meetings m ON m.id = k.meeting_id WHERE k.id = ?1",
    )?;
    let mut get_doc = c.prepare(
        "SELECT k.document_id, d.name, k.text, d.llm_ok
         FROM document_chunks k JOIN documents d ON d.id = k.document_id WHERE k.id = ?1",
    )?;
    ranked
        .into_iter()
        .map(|((doc, id), score)| {
            Ok(if doc {
                get_doc.query_row([id], |r| {
                    Ok(Hit {
                        chunk_id: id,
                        meeting_id: None,
                        document_id: Some(r.get(0)?),
                        title: r.get(1)?,
                        start_ms: 0,
                        end_ms: 0,
                        text: r.get(2)?,
                        score,
                        llm_ok: r.get::<_, i64>(3)? == 1,
                    })
                })?
            } else {
                get_meeting.query_row([id], |r| {
                    Ok(Hit {
                        chunk_id: id,
                        meeting_id: Some(r.get(0)?),
                        document_id: None,
                        title: r.get(1)?,
                        start_ms: r.get(2)?,
                        end_ms: r.get(3)?,
                        text: r.get(4)?,
                        score,
                        llm_ok: true,
                    })
                })?
            })
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
            params![meeting, start_ms, text, to_blob(&HashEmbedder.embed(text).unwrap())],
        )
        .unwrap();
    }

    fn ask(c: &Connection, member: &str, q: &str) -> Vec<Hit> {
        search(c, member, q, &HashEmbedder, 5).unwrap()
    }

    #[test]
    fn a_private_meeting_never_appears_for_another_member() {
        let (_d, c) = setup();
        add(&c, "secret", 0, "the launch budget is nine thousand");
        add(&c, "team", 0, "the launch date moved to May");
        let hits = ask(&c, "alice", "launch budget");
        let ids: Vec<&str> = hits
            .iter()
            .map(|h| h.meeting_id.as_deref().unwrap())
            .collect();
        assert_eq!(
            ids,
            vec!["team"],
            "only the workspace-shared meeting is visible"
        );
        // bob owns it, so he sees both.
        assert_eq!(ask(&c, "bob", "launch budget").len(), 2);
    }

    #[test]
    fn an_attendee_sees_an_attendees_meeting_only_while_it_is_shared_that_way() {
        let (_d, c) = setup();
        add(&c, "secret", 0, "the launch budget is nine thousand");
        // alice is on the invite (email matched case-insensitively) of the event bob's meeting came from.
        c.execute(
            "INSERT INTO calendar_events (id, member_id, external_id, title, starts_at, ends_at, meeting_id, attendees)
             VALUES ('e', 'bob', 'x', 'Sync', 0, 1, 'secret', '[{\"email\":\"Alice@X.test\"}]')",
            [],
        )
        .unwrap();
        assert_eq!(ask(&c, "alice", "launch budget").len(), 1);
        c.execute(
            "UPDATE meetings SET share = 'owner' WHERE id = 'secret'",
            [],
        )
        .unwrap();
        assert_eq!(ask(&c, "alice", "launch budget").len(), 0, "only me");
        assert_eq!(ask(&c, "bob", "launch budget").len(), 1);
    }

    #[test]
    fn a_keyword_only_match_and_a_vector_only_match_both_surface() {
        let (_d, c) = setup();
        add(&c, "mine", 5_000, "pricing page ships on Friday");
        add(&c, "mine", 65_000, "we ordered noodles for lunch");
        let hits = ask(&c, "alice", "when does the pricing page ship");
        assert_eq!(hits[0].start_ms, 5_000);
        assert_eq!(hits[0].meeting_id.as_deref(), Some("mine"));
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

    /// Same vector for every text, so only the embedder id decides whether vectors are compared.
    struct Flat(&'static str);
    impl Embedder for Flat {
        fn id(&self) -> &'static str {
            self.0
        }
        fn embed(&self, _: &str) -> anyhow::Result<Vec<f32>> {
            Ok(vec![1.0, 0.0])
        }
    }

    /// Same vector as `Flat`, but with its own cut-off.
    struct Strict(f32);
    impl Embedder for Strict {
        fn id(&self) -> &'static str {
            "flat-2"
        }
        fn embed(&self, _: &str) -> anyhow::Result<Vec<f32>> {
            Ok(vec![0.6, 0.8])
        }
        fn min_similarity(&self) -> f32 {
            self.0
        }
    }

    #[test]
    fn the_embedders_own_threshold_decides_which_vector_matches_count() {
        let (_d, c) = setup();
        c.execute(
            "INSERT INTO chunks (meeting_id, start_ms, end_ms, text, embedding, embedder) VALUES ('mine', 0, 1000, 'alpha', ?1, 'flat-2')",
            params![to_blob(&[1.0, 0.0])],
        )
        .unwrap();
        // The query vector (0.6, 0.8) has cosine 0.6 with the chunk; no shared word, so only vectors can match.
        assert_eq!(
            search(&c, "alice", "zzz", &Strict(0.55), 5).unwrap().len(),
            1
        );
        assert!(search(&c, "alice", "zzz", &Strict(0.65), 5)
            .unwrap()
            .is_empty());
        assert!(search_at(&c, "alice", "zzz", &Strict(0.55), 5, 0.65)
            .unwrap()
            .is_empty());
        assert_eq!(crate::embed::HashEmbedder.min_similarity(), 0.2);
    }

    struct Down;
    impl Embedder for Down {
        fn id(&self) -> &'static str {
            "flat-2"
        }
        fn embed(&self, _: &str) -> anyhow::Result<Vec<f32>> {
            Err(anyhow::anyhow!("embedder down"))
        }
    }

    #[test]
    fn a_failing_query_embedder_is_an_error_not_an_empty_result() {
        let (_d, c) = setup();
        let err = search(&c, "alice", "alpha", &Down, 5).unwrap_err();
        assert_eq!(err.to_string(), "embedder down");
    }

    #[test]
    fn vectors_are_only_compared_within_one_embedder() {
        let (_d, c) = setup();
        c.execute(
            "INSERT INTO chunks (meeting_id, start_ms, end_ms, text, embedding, embedder) VALUES ('mine', 0, 1000, 'alpha', ?1, 'flat-2')",
            params![to_blob(&[1.0, 0.0])],
        )
        .unwrap();
        // No shared word, so only the vector side can find it.
        let q = "zzz";
        assert_eq!(search(&c, "alice", q, &Flat("flat-2"), 5).unwrap().len(), 1);
        assert!(
            search(&c, "alice", q, &Flat("flat-3"), 5)
                .unwrap()
                .is_empty(),
            "another embedder's vectors are not comparable"
        );
        // Keyword search still reaches the chunk under any embedder.
        assert_eq!(
            search(&c, "alice", "alpha", &Flat("flat-3"), 5)
                .unwrap()
                .len(),
            1
        );
    }

    /// Adds a document for `owner`; the chunk is embedded only when `llm_ok`, as ingest does.
    fn add_doc(c: &Connection, id: &str, owner: &str, llm_ok: bool, text: &str) {
        c.execute(
            "INSERT INTO documents (id, member_id, name, kind, size_bytes, path, llm_ok, created_at)
             VALUES (?1, ?2, ?1 || '.md', 'md', 1, 'p', ?3, 0)",
            params![id, owner, llm_ok as i64],
        )
        .unwrap();
        c.execute(
            "INSERT INTO document_chunks (document_id, seq, text) VALUES (?1, 0, ?2)",
            params![id, text],
        )
        .unwrap();
        if llm_ok {
            c.execute(
                "INSERT INTO document_vectors (chunk_id, document_id, embedder, embedding)
                 VALUES (last_insert_rowid(), ?1, ?2, ?3)",
                params![
                    id,
                    HashEmbedder.id(),
                    to_blob(&HashEmbedder.embed(text).unwrap())
                ],
            )
            .unwrap();
        }
    }

    #[test]
    fn documents_are_visible_to_the_uploader_only() {
        let (_d, c) = setup();
        add_doc(&c, "bobdoc", "bob", true, "zebrafish retainer terms");
        add_doc(&c, "alicedoc", "alice", true, "zebrafish retainer terms");
        let hits = ask(&c, "alice", "zebrafish retainer");
        let ids: Vec<_> = hits.iter().map(|h| h.document_id.as_deref()).collect();
        assert_eq!(ids, vec![Some("alicedoc")]);
        assert_eq!(hits[0].title, "alicedoc.md");
        assert_eq!(hits[0].meeting_id, None);
        let hits = ask(&c, "bob", "zebrafish retainer");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document_id.as_deref(), Some("bobdoc"));
        assert!(ask(&c, "carol", "zebrafish retainer").is_empty());
    }

    #[test]
    fn a_toggle_off_document_is_keyword_only_and_marked() {
        let (_d, c) = setup();
        add_doc(&c, "off", "alice", false, "walrus escrow clause");
        let hits = ask(&c, "alice", "walrus escrow");
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].llm_ok);
        // A vector row planted for a toggle-off document is still never read.
        c.execute(
            "INSERT INTO document_vectors (chunk_id, document_id, embedder, embedding)
             SELECT id, document_id, 'flat-2', ?1 FROM document_chunks",
            params![to_blob(&[1.0, 0.0])],
        )
        .unwrap();
        assert!(search(&c, "alice", "qqq", &Flat("flat-2"), 5)
            .unwrap()
            .is_empty());
        // A toggle-on document is marked sendable; meetings always are.
        add_doc(&c, "on", "alice", true, "walrus escrow clause");
        add(&c, "mine", 0, "walrus escrow clause");
        let mut marks: Vec<_> = ask(&c, "alice", "walrus escrow")
            .iter()
            .map(|h| (h.document_id.clone(), h.llm_ok))
            .collect();
        marks.sort();
        assert_eq!(
            marks,
            vec![
                (None, true),
                (Some("off".into()), false),
                (Some("on".into()), true)
            ]
        );
    }

    #[test]
    fn a_hit_names_its_document_or_its_meeting() {
        let (_d, c) = setup();
        add_doc(&c, "d1", "alice", true, "quokka handbook");
        add(&c, "mine", 3_000, "quokka standup");
        let hits = ask(&c, "alice", "quokka");
        assert_eq!(hits.len(), 2);
        for h in hits {
            assert_eq!(
                h.meeting_id.is_some() as u8 + h.document_id.is_some() as u8,
                1
            );
        }
    }
}
