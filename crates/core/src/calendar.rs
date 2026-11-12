//! Calendar events per member: sync from a provider, the upcoming list and the join switch.
//! Every query is limited to one member, so nobody sees another member's calendar.
use crate::db::new_id;
use anyhow::Result;
use rusqlite::{params, Connection};

pub struct NewEvent {
    pub external_id: String,
    pub title: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub join_url: Option<String>,
    pub platform: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub join_url: Option<String>,
    pub platform: Option<String>,
    pub auto_join: bool,
}

/// Replaces the member's events that start inside `[from, to)` with `events`.
/// Existing events keep their `auto_join` switch and meeting; new ones get `default_auto_join` when
/// they have a join link. Events that vanished from the provider are removed unless a meeting
/// was already created for them.
pub fn sync(
    c: &Connection,
    member_id: &str,
    events: &[NewEvent],
    from: i64,
    to: i64,
    default_auto_join: bool,
) -> Result<()> {
    c.execute_batch("BEGIN")?;
    let done = (|| -> Result<()> {
        for e in events {
            c.execute(
                "INSERT INTO calendar_events
                   (id, member_id, external_id, title, starts_at, ends_at, join_url, platform, auto_join)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (member_id, external_id) DO UPDATE SET
                   title = excluded.title, starts_at = excluded.starts_at, ends_at = excluded.ends_at,
                   join_url = excluded.join_url, platform = excluded.platform,
                   auto_join = CASE WHEN excluded.join_url IS NULL THEN 0 ELSE auto_join END",
                params![
                    new_id(),
                    member_id,
                    e.external_id,
                    e.title,
                    e.starts_at,
                    e.ends_at,
                    e.join_url,
                    e.platform,
                    (default_auto_join && e.join_url.is_some()) as i64
                ],
            )?;
        }
        let mut gone = c.prepare(
            "SELECT id, external_id FROM calendar_events
             WHERE member_id = ?1 AND starts_at >= ?2 AND starts_at < ?3 AND meeting_id IS NULL",
        )?;
        let stale: Vec<String> = gone
            .query_map(params![member_id, from, to], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(_, ext)| !events.iter().any(|e| &e.external_id == ext))
            .map(|(id, _)| id)
            .collect();
        for id in stale {
            c.execute("DELETE FROM calendar_events WHERE id = ?1", [id])?;
        }
        Ok(())
    })();
    c.execute_batch(if done.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    done
}

/// The member's events that have not ended yet, soonest first.
pub fn upcoming(c: &Connection, member_id: &str, now: i64, limit: u32) -> Result<Vec<Event>> {
    let mut s = c.prepare(
        "SELECT id, title, starts_at, ends_at, join_url, platform, auto_join FROM calendar_events
         WHERE member_id = ?1 AND ends_at > ?2 ORDER BY starts_at, id LIMIT ?3",
    )?;
    let rows = s
        .query_map(params![member_id, now, limit], |r| {
            Ok(Event {
                id: r.get(0)?,
                title: r.get(1)?,
                starts_at: r.get(2)?,
                ends_at: r.get(3)?,
                join_url: r.get(4)?,
                platform: r.get(5)?,
                auto_join: r.get::<_, i64>(6)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Flips the join switch. False when the event is not this member's, or has no join link to join.
pub fn set_auto_join(c: &Connection, member_id: &str, event_id: &str, on: bool) -> Result<bool> {
    Ok(c.execute(
        "UPDATE calendar_events SET auto_join = ?3
         WHERE id = ?1 AND member_id = ?2 AND join_url IS NOT NULL",
        params![event_id, member_id, on as i64],
    )? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_tenant;

    fn ev(ext: &str, start: i64, url: Option<&str>) -> NewEvent {
        NewEvent {
            external_id: ext.into(),
            title: format!("title {ext}"),
            starts_at: start,
            ends_at: start + 100,
            join_url: url.map(Into::into),
            platform: url.map(|_| "meet".into()),
        }
    }

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        c.execute_batch(
            "INSERT INTO members (id, email, role) VALUES ('u1','a@x','member'), ('u2','b@x','member');",
        )
        .unwrap();
        (dir, c)
    }

    #[test]
    fn sync_keeps_the_switch_updates_fields_and_drops_vanished_events() {
        let (_d, c) = db();
        let link = Some("https://meet.google.com/abc-defg-hij");
        sync(
            &c,
            "u1",
            &[
                ev("a", 1000, link),
                ev("b", 2000, link),
                ev("c", 3000, None),
            ],
            0,
            10_000,
            false,
        )
        .unwrap();
        let id_a = upcoming(&c, "u1", 0, 10).unwrap()[0].id.clone();
        assert!(set_auto_join(&c, "u1", &id_a, true).unwrap());

        // Second sync: "a" moved and renamed, "b" vanished, "d" is new.
        let mut a = ev("a", 1500, link);
        a.title = "renamed".into();
        sync(
            &c,
            "u1",
            &[a, ev("c", 3000, None), ev("d", 4000, link)],
            0,
            10_000,
            false,
        )
        .unwrap();
        let list = upcoming(&c, "u1", 0, 10).unwrap();
        let got: Vec<(&str, i64, bool)> = list
            .iter()
            .map(|e| (e.title.as_str(), e.starts_at, e.auto_join))
            .collect();
        assert_eq!(
            got,
            vec![
                ("renamed", 1500, true),
                ("title c", 3000, false),
                ("title d", 4000, false)
            ]
        );
        assert_eq!(list[0].id, id_a, "same row, not recreated");
    }

    #[test]
    fn new_events_default_on_only_when_they_have_a_link() {
        let (_d, c) = db();
        sync(
            &c,
            "u1",
            &[
                ev("a", 1000, Some("https://zoom.us/j/123456789")),
                ev("b", 2000, None),
            ],
            0,
            10_000,
            true,
        )
        .unwrap();
        let v: Vec<bool> = upcoming(&c, "u1", 0, 10)
            .unwrap()
            .iter()
            .map(|e| e.auto_join)
            .collect();
        assert_eq!(v, vec![true, false]);
    }

    #[test]
    fn sync_only_touches_the_window_and_events_without_a_meeting() {
        let (_d, c) = db();
        sync(
            &c,
            "u1",
            &[
                ev("old", 500, None),
                ev("in", 1000, None),
                ev("booked", 1100, None),
            ],
            0,
            10_000,
            false,
        )
        .unwrap();
        c.execute("INSERT INTO meetings (id, title, platform, status) VALUES ('m1','t','meet','scheduled')", []).unwrap();
        c.execute(
            "UPDATE calendar_events SET meeting_id='m1' WHERE external_id='booked'",
            [],
        )
        .unwrap();
        // Window starts at 1000: "old" is outside it; "in" vanished; "booked" has a meeting.
        sync(&c, "u1", &[], 1000, 10_000, false).unwrap();
        let names: Vec<String> = upcoming(&c, "u1", 0, 10)
            .unwrap()
            .into_iter()
            .map(|e| e.title)
            .collect();
        assert_eq!(names, vec!["title old", "title booked"]);
    }

    #[test]
    fn upcoming_hides_finished_events_and_respects_the_limit() {
        let (_d, c) = db();
        sync(
            &c,
            "u1",
            &[
                ev("past", 0, None),
                ev("a", 1000, None),
                ev("b", 2000, None),
            ],
            0,
            10_000,
            false,
        )
        .unwrap();
        let v: Vec<String> = upcoming(&c, "u1", 500, 1)
            .unwrap()
            .into_iter()
            .map(|e| e.title)
            .collect();
        assert_eq!(
            v,
            vec!["title a"],
            "'past' ended at 100, the limit keeps one"
        );
    }

    #[test]
    fn one_member_cannot_see_or_flip_another_members_events() {
        let (_d, c) = db();
        sync(
            &c,
            "u1",
            &[ev("a", 1000, Some("https://zoom.us/j/123456789"))],
            0,
            10_000,
            false,
        )
        .unwrap();
        let id = upcoming(&c, "u1", 0, 10).unwrap()[0].id.clone();
        assert!(upcoming(&c, "u2", 0, 10).unwrap().is_empty());
        assert!(!set_auto_join(&c, "u2", &id, true).unwrap());
        assert!(!upcoming(&c, "u1", 0, 10).unwrap()[0].auto_join);
        // Same provider id for two members stays two events.
        sync(&c, "u2", &[ev("a", 1000, None)], 0, 10_000, false).unwrap();
        assert_eq!(upcoming(&c, "u1", 0, 10).unwrap().len(), 1);
        assert_eq!(upcoming(&c, "u2", 0, 10).unwrap().len(), 1);
    }

    #[test]
    fn an_event_without_a_link_cannot_be_switched_on() {
        let (_d, c) = db();
        sync(&c, "u1", &[ev("a", 1000, None)], 0, 10_000, false).unwrap();
        let id = upcoming(&c, "u1", 0, 10).unwrap()[0].id.clone();
        assert!(!set_auto_join(&c, "u1", &id, true).unwrap());
    }

    #[test]
    fn losing_the_join_link_switches_the_event_off_and_keeping_it_does_not() {
        let (_d, c) = db();
        let link = Some("https://meet.google.com/abc-defg-hij");
        sync(&c, "u1", &[ev("a", 1000, link)], 0, 10_000, false).unwrap();
        let id = upcoming(&c, "u1", 0, 10).unwrap()[0].id.clone();
        set_auto_join(&c, "u1", &id, true).unwrap();
        sync(&c, "u1", &[ev("a", 1000, link)], 0, 10_000, false).unwrap();
        assert!(
            upcoming(&c, "u1", 0, 10).unwrap()[0].auto_join,
            "link kept: switch kept"
        );
        sync(&c, "u1", &[ev("a", 1000, None)], 0, 10_000, false).unwrap();
        let e = &upcoming(&c, "u1", 0, 10).unwrap()[0];
        assert_eq!((e.join_url.as_deref(), e.auto_join), (None, false));
    }

    #[test]
    fn deleting_a_meeting_unbooks_the_event_and_deleting_a_member_removes_their_events() {
        let (_d, c) = db();
        sync(&c, "u1", &[ev("a", 1000, None)], 0, 10_000, false).unwrap();
        sync(&c, "u2", &[ev("a", 1000, None)], 0, 10_000, false).unwrap();
        c.execute("INSERT INTO meetings (id, title, platform, status) VALUES ('m1','t','meet','scheduled')", []).unwrap();
        c.execute(
            "UPDATE calendar_events SET meeting_id='m1' WHERE member_id='u1'",
            [],
        )
        .unwrap();
        c.execute("DELETE FROM meetings WHERE id='m1'", []).unwrap();
        let booked: Option<String> = c
            .query_row(
                "SELECT meeting_id FROM calendar_events WHERE member_id='u1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(booked, None);
        c.execute("DELETE FROM members WHERE id='u1'", []).unwrap();
        assert!(upcoming(&c, "u1", 0, 10).unwrap().is_empty());
        assert_eq!(
            upcoming(&c, "u2", 0, 10).unwrap().len(),
            1,
            "the other member's event stays"
        );
    }

    #[test]
    fn a_running_event_is_still_upcoming_and_a_moved_event_leaves_the_window_alive() {
        let (_d, c) = db();
        sync(&c, "u1", &[ev("a", 1000, None)], 0, 10_000, false).unwrap(); // runs 1000..1100
        assert_eq!(
            upcoming(&c, "u1", 1050, 10).unwrap().len(),
            1,
            "started but not ended"
        );
        assert_eq!(
            upcoming(&c, "u1", 1100, 10).unwrap().len(),
            0,
            "ended exactly now"
        );
        // The event moves outside the sync window: it is updated, not deleted.
        sync(&c, "u1", &[ev("a", 50_000, None)], 0, 10_000, false).unwrap();
        assert_eq!(upcoming(&c, "u1", 0, 10).unwrap()[0].starts_at, 50_000);
    }
}
