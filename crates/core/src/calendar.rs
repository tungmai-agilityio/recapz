//! Calendar events per member: sync from a provider, the upcoming list and the join switch.
//! Every query is limited to one member, so nobody sees another member's calendar.
use crate::db::new_id;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

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
    c.execute_batch("BEGIN IMMEDIATE")?;
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

/// Creates a `scheduled` meeting (private to its owner) for every switched-on event that has a join
/// link, no meeting yet, and starts within `lead_ms` from `now` or is already running. An active
/// meeting on the same link (any owner) is reused, so one call gets one notetaker; its owner is the
/// first member to book it and the others get no access to it. Returns the new meeting ids.
pub fn book_due(c: &Connection, now: i64, lead_ms: i64) -> Result<Vec<String>> {
    c.execute_batch("BEGIN IMMEDIATE")?;
    let done = (|| -> Result<Vec<String>> {
        let mut q = c.prepare(
            "SELECT id, member_id, title, join_url, platform FROM calendar_events
             WHERE auto_join = 1 AND join_url IS NOT NULL AND meeting_id IS NULL
               AND starts_at <= ?1 AND ends_at > ?2 ORDER BY starts_at, id",
        )?;
        let due = q
            .query_map(params![now + lead_ms, now], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut created = Vec::new();
        for (event_id, member, title, url, platform) in due {
            // One notetaker per call: an active meeting on this link is reused whoever owns it.
            let existing: Option<String> = c
                .query_row(
                    "SELECT id FROM meetings WHERE join_url = ?1
                       AND status IN ('scheduled','joining','recording') ORDER BY created_at, id LIMIT 1",
                    params![url],
                    |r| r.get(0),
                )
                .optional()?;
            let meeting = match existing {
                Some(id) => id,
                None => {
                    let id = new_id();
                    c.execute(
                        "INSERT INTO meetings (id, title, platform, join_url, status, owner_id, created_at)
                         VALUES (?1, ?2, ?3, ?4, 'scheduled', ?5, ?6)",
                        params![id, title, platform.unwrap_or_default(), url, member, now],
                    )?;
                    created.push(id.clone());
                    id
                }
            };
            c.execute(
                "UPDATE calendar_events SET meeting_id = ?2 WHERE id = ?1",
                params![event_id, meeting],
            )?;
        }
        Ok(created)
    })();
    c.execute_batch(if done.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    done
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

    fn switched_on(c: &Connection, member: &str, ext: &str, start: i64, url: &str) {
        sync(c, member, &[ev(ext, start, Some(url))], 0, 1_000_000, true).unwrap();
    }

    fn meetings(c: &Connection) -> Vec<(String, String, String, String)> {
        c.prepare("SELECT title, platform, status, owner_id FROM meetings ORDER BY title")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn books_a_private_scheduled_meeting_once_within_the_lead_time() {
        let (_d, c) = db();
        let url = "https://meet.google.com/abc-defg-hij";
        switched_on(&c, "u1", "a", 10_000, url); // runs 10_000..10_100
        assert!(
            book_due(&c, 4_000, 5_000).unwrap().is_empty(),
            "starts in 6 s, lead is 5 s"
        );
        let ids = book_due(&c, 5_000, 5_000).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(
            meetings(&c),
            vec![(
                "title a".to_string(),
                "meet".to_string(),
                "scheduled".to_string(),
                "u1".to_string()
            )]
        );
        let share: String = c
            .query_row("SELECT share FROM meetings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(share, "attendees", "private to the owner unless shared");
        assert!(
            book_due(&c, 6_000, 5_000).unwrap().is_empty(),
            "booked events are not booked again"
        );
        assert_eq!(meetings(&c).len(), 1);
        let linked: Option<String> = c
            .query_row("SELECT meeting_id FROM calendar_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(linked.as_deref(), Some(ids[0].as_str()));
    }

    #[test]
    fn only_switched_on_events_with_a_link_that_have_not_ended_are_booked() {
        let (_d, c) = db();
        let url = "https://zoom.us/j/123456789";
        sync(
            &c,
            "u1",
            &[
                ev("off", 1000, Some(url)),
                ev("nolink", 1000, None),
                ev("past", 0, Some(url)),
                ev("on", 1050, Some(url)),
            ],
            0,
            10_000,
            false,
        )
        .unwrap();
        let on_id = upcoming(&c, "u1", 0, 10)
            .unwrap()
            .into_iter()
            .find(|e| e.title == "title on")
            .unwrap()
            .id;
        set_auto_join(&c, "u1", &on_id, true).unwrap();
        let past_id: String = c
            .query_row(
                "SELECT id FROM calendar_events WHERE external_id='past'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        c.execute(
            "UPDATE calendar_events SET auto_join = 1 WHERE id = ?1",
            [past_id],
        )
        .unwrap(); // ended at 100
        let ids = book_due(&c, 1060, 0).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(
            meetings(&c)[0].0,
            "title on",
            "a running event is still joined"
        );
    }

    #[test]
    fn a_meeting_the_member_already_queued_for_the_link_is_reused() {
        let (_d, c) = db();
        let url = "https://meet.google.com/abc-defg-hij";
        c.execute(
            "INSERT INTO meetings (id, title, platform, join_url, status, owner_id) VALUES ('m1','pasted','meet',?1,'scheduled','u1')",
            [url],
        )
        .unwrap();
        switched_on(&c, "u1", "a", 1000, url);
        assert!(
            book_due(&c, 1000, 0).unwrap().is_empty(),
            "nothing new is created"
        );
        assert_eq!(meetings(&c).len(), 1);
        let linked: Option<String> = c
            .query_row("SELECT meeting_id FROM calendar_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(linked.as_deref(), Some("m1"));
    }

    #[test]
    fn one_call_gets_one_meeting_even_when_several_members_switch_it_on() {
        let (_d, c) = db();
        let url = "https://meet.google.com/abc-defg-hij";
        switched_on(&c, "u1", "a", 1000, url);
        switched_on(&c, "u2", "a", 1000, url);
        assert_eq!(book_due(&c, 1000, 0).unwrap().len(), 1);
        let all = meetings(&c);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].3, "u1", "the first member to book owns it");
        let linked: Vec<Option<String>> = c
            .prepare("SELECT meeting_id FROM calendar_events ORDER BY member_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(linked.len(), 2);
        assert!(
            linked.iter().all(|m| m == &linked[0] && m.is_some()),
            "both events point at the one meeting"
        );
    }

    #[test]
    fn two_events_on_one_link_in_one_run_make_one_meeting() {
        let (_d, c) = db();
        let url = "https://zoom.us/j/123456789";
        sync(
            &c,
            "u1",
            &[ev("a", 1000, Some(url)), ev("b", 1000, Some(url))],
            0,
            10_000,
            true,
        )
        .unwrap();
        assert_eq!(book_due(&c, 1000, 0).unwrap().len(), 1);
        assert_eq!(meetings(&c).len(), 1);
    }

    #[test]
    fn a_finished_meeting_on_the_link_does_not_block_a_new_booking() {
        let (_d, c) = db();
        let url = "https://meet.google.com/abc-defg-hij";
        c.execute(
            "INSERT INTO meetings (id, title, platform, join_url, status, owner_id) VALUES ('old','old','meet',?1,'ready','u2')",
            [url],
        )
        .unwrap();
        switched_on(&c, "u1", "a", 1000, url);
        assert_eq!(book_due(&c, 1000, 0).unwrap().len(), 1);
        assert_eq!(meetings(&c).len(), 2);
    }
}
