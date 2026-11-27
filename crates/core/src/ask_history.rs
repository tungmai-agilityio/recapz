//! A member's Ask history. Every function takes the member and never touches another member's turns.
//! A turn cites meetings and documents in `citations_json` (the cards the answer showed); purging a
//! meeting or a document deletes the turns that cite it (`purge_citing`, called inside the purge
//! transaction), and removing a member deletes theirs.
use crate::db::{new_id, now_ms};
use anyhow::Result;
use rusqlite::{params, Connection};

/// Turns kept per member; saving past it drops the oldest.
pub const CAP: i64 = 200;

#[derive(Debug, PartialEq)]
pub struct Turn {
    pub id: String,
    pub question: String,
    pub answer: String,
    pub citations_json: String,
    pub created_at: i64,
}

/// Saves a turn and prunes the member's oldest beyond `CAP`. Returns the new turn's id.
pub fn save(
    c: &Connection,
    member_id: &str,
    question: &str,
    answer: &str,
    citations_json: &str,
) -> Result<String> {
    let id = new_id();
    c.execute(
        "INSERT INTO ask_turns (id, member_id, question, answer, citations_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, member_id, question, answer, citations_json, now_ms()],
    )?;
    c.execute(
        "DELETE FROM ask_turns WHERE member_id = ?1 AND id NOT IN
           (SELECT id FROM ask_turns WHERE member_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2)",
        params![member_id, CAP],
    )?;
    Ok(id)
}

/// The member's turns, oldest first.
pub fn list(c: &Connection, member_id: &str) -> Result<Vec<Turn>> {
    let mut s = c.prepare(
        "SELECT id, question, answer, citations_json, created_at FROM ask_turns
         WHERE member_id = ?1 ORDER BY created_at, id",
    )?;
    let rows = s.query_map([member_id], |r| {
        Ok(Turn {
            id: r.get(0)?,
            question: r.get(1)?,
            answer: r.get(2)?,
            citations_json: r.get(3)?,
            created_at: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Deletes one of the member's own turns; false when it is missing or another member's.
pub fn delete_one(c: &Connection, member_id: &str, id: &str) -> Result<bool> {
    Ok(c.execute(
        "DELETE FROM ask_turns WHERE id = ?1 AND member_id = ?2",
        [id, member_id],
    )? > 0)
}

/// Deletes all of the member's turns; returns how many.
pub fn clear(c: &Connection, member_id: &str) -> Result<usize> {
    Ok(c.execute("DELETE FROM ask_turns WHERE member_id = ?1", [member_id])?)
}

/// Deletes every member's turns that cite this meeting (`key` = `meeting_id`) or document
/// (`key` = `document_id`). Callers run it in the purge transaction.
pub(crate) fn purge_citing(c: &Connection, key: &str, id: &str) -> Result<usize> {
    debug_assert!(key == "meeting_id" || key == "document_id");
    Ok(c.execute(
        "DELETE FROM ask_turns WHERE EXISTS (SELECT 1 FROM json_each(ask_turns.citations_json)
           WHERE json_extract(value, ?1) = ?2)",
        [&format!("$.{key}"), id],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_tenant;

    fn cite(m: &str, d: &str) -> String {
        let f = |k: &str, v: &str| {
            if v.is_empty() {
                format!("\"{k}\":null")
            } else {
                format!("\"{k}\":\"{v}\"")
            }
        };
        format!("[{{{},{}}}]", f("meeting_id", m), f("document_id", d))
    }

    #[test]
    fn saves_lists_oldest_first_and_keeps_members_apart() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        let a = save(&c, "u1", "q1", "a1", "[]").unwrap();
        save(&c, "u1", "q2", "a2", &cite("m1", "")).unwrap();
        save(&c, "u2", "other", "x", "[]").unwrap();
        let t = list(&c, "u1").unwrap();
        assert_eq!(
            t.iter().map(|t| t.question.as_str()).collect::<Vec<_>>(),
            ["q1", "q2"]
        );
        assert_eq!(t[1].citations_json, cite("m1", ""));
        assert_eq!(list(&c, "u2").unwrap().len(), 1);
        assert!(!delete_one(&c, "u2", &a).unwrap(), "another member's turn");
        assert_eq!(list(&c, "u1").unwrap().len(), 2);
        assert!(delete_one(&c, "u1", &a).unwrap());
        assert!(!delete_one(&c, "u1", &a).unwrap(), "already gone");
        assert_eq!(clear(&c, "u1").unwrap(), 1);
        assert_eq!(list(&c, "u2").unwrap().len(), 1, "clear is per member");
    }

    #[test]
    fn keeps_the_newest_cap_turns_per_member() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        for i in 0..CAP + 3 {
            save(&c, "u1", &format!("q{i}"), "a", "[]").unwrap();
        }
        save(&c, "u2", "keep", "a", "[]").unwrap();
        let t = list(&c, "u1").unwrap();
        assert_eq!(t.len() as i64, CAP);
        assert_eq!(t[0].question, "q3");
        assert_eq!(t.last().unwrap().question, format!("q{}", CAP + 2));
        assert_eq!(list(&c, "u2").unwrap().len(), 1);
    }

    #[test]
    fn purge_citing_matches_only_the_named_meeting_or_document() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        save(&c, "u1", "m1", "a", &cite("m1", "")).unwrap();
        save(&c, "u2", "m1 too", "a", &cite("m1", "")).unwrap();
        save(&c, "u1", "m2", "a", &cite("m2", "")).unwrap();
        save(&c, "u1", "d1", "a", &cite("", "d1")).unwrap();
        save(&c, "u1", "none", "a", "[]").unwrap();
        assert_eq!(purge_citing(&c, "meeting_id", "m1").unwrap(), 2);
        assert_eq!(purge_citing(&c, "document_id", "d1").unwrap(), 1);
        assert_eq!(purge_citing(&c, "meeting_id", "d1").unwrap(), 0);
        assert_eq!(
            list(&c, "u1")
                .unwrap()
                .iter()
                .map(|t| t.question.as_str())
                .collect::<Vec<_>>(),
            ["m2", "none"]
        );
        assert!(list(&c, "u2").unwrap().is_empty());
    }
}
