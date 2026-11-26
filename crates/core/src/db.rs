use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use std::path::{Path, PathBuf};

/// The one meeting access rule, for a query over `meetings m` where `?1` is the member id: the owner,
/// anyone when shared with the workspace, or (the default `attendees`) a member whose email is on the
/// invite of the calendar event the meeting came from. `owner` shares with nobody else.
pub const MEETING_VISIBLE: &str = "(m.owner_id = ?1 OR m.share = 'workspace' OR (m.share = 'attendees' AND EXISTS (
    SELECT 1 FROM calendar_events e, json_each(e.attendees) a
    WHERE e.meeting_id = m.id AND lower(json_extract(a.value, '$.email')) = (SELECT lower(email) FROM members WHERE id = ?1))))";

pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn open(path: &Path, migrations: &Migrations) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    migrations.to_latest(&mut conn)?;
    Ok(conn)
}

/// Opens (and migrates) `data/control.sqlite`: tenants, users, memberships, invites, sessions.
pub fn open_control(data_dir: &Path) -> Result<Connection> {
    let m = Migrations::new(vec![
        M::up(include_str!("../migrations/control_001.sql")),
        M::up(include_str!("../migrations/control_002.sql")),
        M::up(include_str!("../migrations/control_003.sql")),
    ]);
    open(&data_dir.join("control.sqlite"), &m)
}

pub fn tenant_dir(data_dir: &Path, tenant_id: &str) -> PathBuf {
    data_dir.join("tenants").join(tenant_id)
}

/// Opens (and migrates) one tenant database. Migrations are additive only.
pub fn open_tenant(data_dir: &Path, tenant_id: &str) -> Result<Connection> {
    let m = Migrations::new(vec![
        M::up(include_str!("../migrations/tenant_001.sql")),
        M::up(include_str!("../migrations/tenant_002.sql")),
        M::up(include_str!("../migrations/tenant_003.sql")),
        M::up(include_str!("../migrations/tenant_004.sql")),
        M::up(include_str!("../migrations/tenant_005.sql")),
        M::up(include_str!("../migrations/tenant_006.sql")),
        M::up(include_str!("../migrations/tenant_007.sql")),
        M::up(include_str!("../migrations/tenant_008.sql")),
    ]);
    open(&tenant_dir(data_dir, tenant_id).join("recapz.sqlite"), &m)
}

pub fn tenant_ids(control: &Connection) -> Result<Vec<String>> {
    let mut s = control.prepare("SELECT id FROM tenants ORDER BY id")?;
    let ids = s.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(ids)
}

/// Creates the first tenant and admin when the control DB has no tenants.
pub fn bootstrap_admin(control: &Connection, email: &str) -> Result<()> {
    if !tenant_ids(control)?.is_empty() {
        return Ok(());
    }
    let tid = new_id();
    control.execute(
        "INSERT INTO tenants (id, name) VALUES (?1, 'Workspace')",
        [&tid],
    )?;
    control.execute(
        "INSERT INTO workspace_invites (email, tenant_id, role, invited_by, created_at)
         VALUES (?1, ?2, 'admin', NULL, ?3)",
        params![email.to_lowercase(), tid, now_ms()],
    )?;
    Ok(())
}

/// Adds a workspace for the server owner: the tenant row, its migrated database and an admin invite for
/// `admin_email`. A name already in use (any case) is refused. Returns the new workspace id.
pub fn create_workspace(
    control: &Connection,
    data_dir: &Path,
    name: &str,
    admin_email: &str,
) -> Result<String> {
    let (name, email) = (name.trim(), admin_email.trim().to_lowercase());
    if name.is_empty() {
        anyhow::bail!("the workspace name is empty");
    }
    if !email.contains('@') {
        anyhow::bail!("'{admin_email}' is not an email address");
    }
    let taken: bool = control.query_row(
        "SELECT EXISTS (SELECT 1 FROM tenants WHERE lower(name) = lower(?1))",
        [name],
        |r| r.get(0),
    )?;
    if taken {
        anyhow::bail!("a workspace named '{name}' already exists");
    }
    let id = new_id();
    open_tenant(data_dir, &id)?;
    let added = control
        .execute(
            "INSERT INTO tenants (id, name) VALUES (?1, ?2)",
            [&id, name],
        )
        .and_then(|_| {
            control.execute(
                "INSERT INTO workspace_invites (email, tenant_id, role, invited_by, created_at)
                 VALUES (?1, ?2, 'admin', NULL, ?3)",
                params![email, id, now_ms()],
            )
        });
    if let Err(e) = added {
        // Do not leave a tenant directory that no tenants row points to.
        let _ = std::fs::remove_dir_all(tenant_dir(data_dir, &id));
        let _ = control.execute("DELETE FROM tenants WHERE id = ?1", [&id]);
        return Err(e.into());
    }
    Ok(id)
}

/// Sets one workspace's monthly cap (micro-USD; 0 = no cap), overriding the config default.
pub fn set_workspace_cap(control: &Connection, id: &str, cap_micro_usd: u64) -> Result<()> {
    let n = control.execute(
        "UPDATE tenants SET monthly_cap_micro_usd = ?2 WHERE id = ?1",
        params![id, cap_micro_usd as i64],
    )?;
    if n == 0 {
        anyhow::bail!("no workspace with id '{id}'");
    }
    Ok(())
}

/// The cap that applies to a workspace: its own when the server owner set one, else `default`.
pub fn effective_cap(control: &Connection, id: &str, default: u64) -> Result<u64> {
    let own: Option<i64> = control.query_row(
        "SELECT monthly_cap_micro_usd FROM tenants WHERE id = ?1",
        [id],
        |r| r.get(0),
    )?;
    Ok(own.map_or(default, |v| v.max(0) as u64))
}

/// This month's estimated cost summed over every workspace.
pub fn server_month_total(control: &Connection, data_dir: &Path, now: i64) -> Result<i64> {
    let mut sum = 0;
    for id in tenant_ids(control)? {
        sum += crate::usage::month_totals(&open_tenant(data_dir, &id)?, now)?.total();
    }
    Ok(sum)
}

pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub members: i64,
    /// This month's estimated cost in micro-USD.
    pub cost_micro_usd: i64,
}

/// Every workspace with its member count and this month's cost.
pub fn list_workspaces(
    control: &Connection,
    data_dir: &Path,
    now: i64,
) -> Result<Vec<WorkspaceRow>> {
    let mut s = control.prepare(
        "SELECT t.id, t.name, (SELECT count(*) FROM memberships m WHERE m.tenant_id = t.id)
         FROM tenants t ORDER BY t.name, t.id",
    )?;
    let rows = s
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<(String, String, i64)>, _>>()?;
    rows.into_iter()
        .map(|(id, name, members)| {
            let cost = crate::usage::month_totals(&open_tenant(data_dir, &id)?, now)?.total();
            Ok(WorkspaceRow {
                id,
                name,
                members,
                cost_micro_usd: cost,
            })
        })
        .collect()
}

pub struct User {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub tenant_id: String,
    pub role: String,
}

pub(crate) const INVITE_TTL_MS: i64 = 14 * 24 * 60 * 60 * 1000;

/// The workspace a person lands in: `users.tenant_id` while it is still a membership, else the oldest one.
fn pick_tenant(control: &Connection, user_id: &str) -> Result<Option<String>> {
    Ok(control
        .query_row(
            "SELECT m.tenant_id FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.user_id = ?1
             ORDER BY (m.tenant_id = u.tenant_id) DESC, m.created_at, m.tenant_id LIMIT 1",
            [user_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Invite-only sign-in: an existing user, or a pending invite which becomes a user. Every unexpired invite
/// for the email (14 days) is accepted as a membership. Returns None for anyone else.
/// `invites` is the pre-workspaces table: its rows never expire and are accepted the same way.
pub fn user_for_sign_in(
    control: &Connection,
    email: &str,
    name: Option<&str>,
) -> Result<Option<User>> {
    let email = email.to_lowercase();
    let cutoff = now_ms() - INVITE_TTL_MS;
    let invites: Vec<(String, String)> = control
        .prepare(
            "SELECT tenant_id, role FROM (
               SELECT tenant_id, role, created_at FROM workspace_invites WHERE email = ?1 AND created_at > ?2
               UNION ALL SELECT tenant_id, role, 0 FROM invites WHERE email = ?1)
             ORDER BY created_at, tenant_id",
        )?
        .query_map(params![email, cutoff], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let existing: Option<String> = control
        .query_row("SELECT id FROM users WHERE email = ?1", [&email], |r| {
            r.get(0)
        })
        .optional()?;
    let user_id = match existing {
        Some(id) => id,
        None => {
            let Some((tenant_id, role)) = invites.first() else {
                return Ok(None);
            };
            let id = new_id();
            control.execute(
                "INSERT INTO users (id, email, name, tenant_id, role) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, email, name, tenant_id, role],
            )?;
            id
        }
    };
    for (tenant_id, role) in &invites {
        control.execute(
            "INSERT OR IGNORE INTO memberships (user_id, tenant_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![user_id, tenant_id, role, now_ms()],
        )?;
    }
    control.execute(
        "DELETE FROM workspace_invites WHERE email = ?1 AND created_at > ?2",
        params![email, cutoff],
    )?;
    control.execute("DELETE FROM invites WHERE email = ?1", [&email])?;
    let Some(tenant_id) = pick_tenant(control, &user_id)? else {
        return Ok(None);
    };
    Ok(control
        .query_row(
            "SELECT u.id, u.email, u.name, m.tenant_id, m.role FROM users u JOIN memberships m ON m.user_id = u.id
             WHERE u.id = ?1 AND m.tenant_id = ?2",
            params![user_id, tenant_id],
            |r| Ok(User { id: r.get(0)?, email: r.get(1)?, name: r.get(2)?, tenant_id: r.get(3)?, role: r.get(4)? }),
        )
        .optional()?)
}

/// Starts a session in the person's current workspace (see `pick_tenant`).
pub fn create_session(control: &Connection, user_id: &str, ttl_ms: i64) -> Result<String> {
    let id = format!("{}{}", new_id(), new_id()).replace('-', "");
    control.execute(
        "INSERT INTO sessions (id, user_id, expires_at, tenant_id) VALUES (?1, ?2, ?3, ?4)",
        params![
            id,
            user_id,
            now_ms() + ttl_ms,
            pick_tenant(control, user_id)?
        ],
    )?;
    Ok(id)
}

/// The session's workspace and the role from its membership; None once the membership is gone.
/// A session made before workspaces has no tenant and uses `users.tenant_id`.
pub fn user_for_session(control: &Connection, session_id: &str) -> Result<Option<User>> {
    Ok(control
        .query_row(
            "SELECT u.id, u.email, u.name, m.tenant_id, m.role FROM sessions s JOIN users u ON u.id = s.user_id
             JOIN memberships m ON m.user_id = u.id AND m.tenant_id = COALESCE(s.tenant_id, u.tenant_id)
             WHERE s.id = ?1 AND s.expires_at > ?2",
            params![session_id, now_ms()],
            |r| Ok(User { id: r.get(0)?, email: r.get(1)?, name: r.get(2)?, tenant_id: r.get(3)?, role: r.get(4)? }),
        )
        .optional()?)
}

pub fn delete_session(control: &Connection, session_id: &str) -> Result<()> {
    control.execute("DELETE FROM sessions WHERE id = ?1", [session_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_control_and_tenant_and_keeps_search_in_sync() {
        let dir = tempfile::tempdir().unwrap();
        let control = open_control(dir.path()).unwrap();
        control
            .execute("INSERT INTO tenants (id, name) VALUES ('t1', 'Team')", [])
            .unwrap();
        assert_eq!(tenant_ids(&control).unwrap(), vec!["t1"]);

        let t = open_tenant(dir.path(), "t1").unwrap();
        assert!(tenant_dir(dir.path(), "t1").join("recapz.sqlite").exists());
        t.execute_batch(
            "INSERT INTO meetings (id, title, platform, status) VALUES ('m1', 'Sync', 'upload', 'ready');
             INSERT INTO chunks (id, meeting_id, text, embedding) VALUES (1, 'm1', 'họp tuần', x'00');",
        )
        .unwrap();
        let hit = |q: &str| -> i64 {
            t.query_row(
                "SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH ?1",
                [q],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(hit("hop"), 1, "diacritics are folded");
        // Deleting a meeting hard-deletes its chunks and their search rows.
        t.execute("DELETE FROM meetings WHERE id = 'm1'", [])
            .unwrap();
        assert_eq!(hit("hop"), 0);
        // Reopening is a no-op migration.
        open_tenant(dir.path(), "t1").unwrap();
    }

    #[test]
    fn migrations_and_invite_only_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let control = open_control(dir.path()).unwrap();
        bootstrap_admin(&control, "Boss@Example.com").unwrap();
        bootstrap_admin(&control, "other@example.com").unwrap(); // no-op, tenant exists
        assert!(user_for_sign_in(&control, "stranger@example.com", None)
            .unwrap()
            .is_none());
        let u = user_for_sign_in(&control, "boss@example.com", Some("Boss"))
            .unwrap()
            .unwrap();
        assert_eq!(u.role, "admin");
        let sid = create_session(&control, &u.id, 60_000).unwrap();
        assert_eq!(
            user_for_session(&control, &sid).unwrap().unwrap().email,
            "boss@example.com"
        );
        delete_session(&control, &sid).unwrap();
        assert!(user_for_session(&control, &sid).unwrap().is_none());

        let t = open_tenant(dir.path(), &u.tenant_id).unwrap();
        let n: i64 = t
            .query_row("SELECT count(*) FROM meetings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    fn control_with_two_tenants() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let c = open_control(dir.path()).unwrap();
        c.execute_batch(
            "INSERT INTO tenants (id, name) VALUES ('t1', 'One'), ('t2', 'Two'), ('t3', 'Three');",
        )
        .unwrap();
        (dir, c)
    }

    #[test]
    fn upgrade_keeps_workspace_role_and_session() {
        let dir = tempfile::tempdir().unwrap();
        let old = Migrations::new(vec![M::up(include_str!("../migrations/control_001.sql"))]);
        let c = open(&dir.path().join("control.sqlite"), &old).unwrap();
        c.execute_batch(
            "INSERT INTO tenants (id, name) VALUES ('t1', 'One'), ('t2', 'Two');
             INSERT INTO users (id, email, tenant_id, role) VALUES ('u1', 'a@x.com', 't1', 'admin'), ('u2', 'b@x.com', 't2', 'member');
             INSERT INTO invites (email, tenant_id, role) VALUES ('c@x.com', 't2', 'member');
             INSERT INTO sessions (id, user_id, expires_at) VALUES ('s1', 'u1', 9999999999999), ('s2', 'u2', 9999999999999);",
        )
        .unwrap();
        drop(c);
        let c = open_control(dir.path()).unwrap();
        let s1 = user_for_session(&c, "s1").unwrap().unwrap();
        assert_eq!((s1.tenant_id.as_str(), s1.role.as_str()), ("t1", "admin"));
        let s2 = user_for_session(&c, "s2").unwrap().unwrap();
        assert_eq!((s2.tenant_id.as_str(), s2.role.as_str()), ("t2", "member"));
        let n: i64 = c
            .query_row("SELECT count(*) FROM memberships", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
        // The old invite is still there, copied, and can be accepted.
        let u = user_for_sign_in(&c, "c@x.com", None).unwrap().unwrap();
        assert_eq!((u.tenant_id.as_str(), u.role.as_str()), ("t2", "member"));
    }

    #[test]
    fn two_workspace_user_gets_the_one_used_last_and_old_invites_are_ignored() {
        let (_d, c) = control_with_two_tenants();
        let now = now_ms();
        c.execute(
            "INSERT INTO workspace_invites (email, tenant_id, role, created_at) VALUES ('a@x.com', 't1', 'admin', ?1)",
            [now - 1000],
        )
        .unwrap();
        let u = user_for_sign_in(&c, "a@x.com", None).unwrap().unwrap();
        assert_eq!(u.tenant_id, "t1");
        // A second invite arrives; t1 stays current while it is users.tenant_id.
        c.execute(
            "INSERT INTO workspace_invites (email, tenant_id, role, created_at) VALUES ('a@x.com', 't2', 'member', ?1)",
            [now],
        )
        .unwrap();
        let u = user_for_sign_in(&c, "a@x.com", None).unwrap().unwrap();
        assert_eq!((u.tenant_id.as_str(), u.role.as_str()), ("t1", "admin"));
        // Switching (done by a later ticket) sets users.tenant_id; the next sign-in lands there.
        c.execute("UPDATE users SET tenant_id = 't2' WHERE id = ?1", [&u.id])
            .unwrap();
        let u = user_for_sign_in(&c, "a@x.com", None).unwrap().unwrap();
        assert_eq!((u.tenant_id.as_str(), u.role.as_str()), ("t2", "member"));
        let sid = create_session(&c, &u.id, 60_000).unwrap();
        assert_eq!(user_for_session(&c, &sid).unwrap().unwrap().tenant_id, "t2");

        // An invite older than 14 days is ignored, for a new person and for an existing one.
        let old = now - INVITE_TTL_MS - 1;
        c.execute(
            "INSERT INTO workspace_invites (email, tenant_id, role, created_at) VALUES ('new@x.com', 't1', 'member', ?1)",
            [old],
        )
        .unwrap();
        assert!(user_for_sign_in(&c, "new@x.com", None).unwrap().is_none());
        // An expired invite for an existing person adds nothing.
        c.execute(
            "INSERT INTO workspace_invites (email, tenant_id, role, created_at) VALUES ('a@x.com', 't3', 'member', ?1)",
            [old],
        )
        .unwrap();
        user_for_sign_in(&c, "a@x.com", None).unwrap();
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM memberships WHERE user_id = ?1",
                [&u.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn removing_the_membership_signs_the_session_out() {
        let (_d, c) = control_with_two_tenants();
        c.execute(
            "INSERT INTO workspace_invites (email, tenant_id, role, created_at) VALUES ('a@x.com', 't1', 'member', ?1)",
            [now_ms()],
        )
        .unwrap();
        let u = user_for_sign_in(&c, "a@x.com", None).unwrap().unwrap();
        let sid = create_session(&c, &u.id, 60_000).unwrap();
        assert!(user_for_session(&c, &sid).unwrap().is_some());
        c.execute("DELETE FROM memberships WHERE user_id = ?1", [&u.id])
            .unwrap();
        assert!(user_for_session(&c, &sid).unwrap().is_none());
    }

    #[test]
    fn workspace_create_refuses_duplicates_and_list_counts_members_and_cost() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_control(dir.path()).unwrap();
        let id = create_workspace(&c, dir.path(), " Acme ", "Boss@X.com").unwrap();
        assert!(tenant_dir(dir.path(), &id).join("recapz.sqlite").exists());
        let err = create_workspace(&c, dir.path(), "ACME", "other@x.com").unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        assert!(create_workspace(&c, dir.path(), "", "a@x.com").is_err());
        assert!(create_workspace(&c, dir.path(), "Beta", "nope").is_err());
        assert_eq!(tenant_ids(&c).unwrap(), vec![id.clone()]);
        // The admin invite works: signing in makes the person an admin of the new workspace.
        let u = user_for_sign_in(&c, "boss@x.com", None).unwrap().unwrap();
        assert_eq!(
            (u.tenant_id.as_str(), u.role.as_str()),
            (id.as_str(), "admin")
        );

        let id2 = create_workspace(&c, dir.path(), "Beta", "b@x.com").unwrap();
        let t = open_tenant(dir.path(), &id).unwrap();
        let now = now_ms();
        t.execute_batch(
            "INSERT INTO meetings (id, title, platform, status) VALUES ('m', 'Sync', 'upload', 'ready');",
        )
        .unwrap();
        t.execute(
            "INSERT INTO speech_usage (meeting_id, kind, audio_seconds, cost_micro_usd, at) VALUES ('m', 'batch', 60, 12000, ?1)",
            [now],
        )
        .unwrap();
        let rows = list_workspaces(&c, dir.path(), now).unwrap();
        let got: Vec<_> = rows
            .iter()
            .map(|w| (w.name.as_str(), w.members, w.cost_micro_usd))
            .collect();
        assert_eq!(got, vec![("Acme", 1, 12000), ("Beta", 0, 0)]);
        assert_eq!(rows[1].id, id2);
    }

    #[test]
    fn workspace_cap_overrides_the_default_and_rejects_an_unknown_id() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_control(dir.path()).unwrap();
        let a = create_workspace(&c, dir.path(), "A", "a@x.com").unwrap();
        let b = create_workspace(&c, dir.path(), "B", "b@x.com").unwrap();
        assert_eq!(effective_cap(&c, &a, 7).unwrap(), 7);
        set_workspace_cap(&c, &a, 42).unwrap();
        assert_eq!(effective_cap(&c, &a, 7).unwrap(), 42);
        assert_eq!(effective_cap(&c, &b, 7).unwrap(), 7);
        set_workspace_cap(&c, &a, 0).unwrap(); // 0 = no cap, not "use the default"
        assert_eq!(effective_cap(&c, &a, 7).unwrap(), 0);
        assert!(set_workspace_cap(&c, "nope", 1).is_err());
    }

    #[test]
    fn a_failed_tenants_insert_leaves_no_tenant_directory() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_control(dir.path()).unwrap();
        c.execute_batch(
            "CREATE TRIGGER no_tenants BEFORE INSERT ON tenants BEGIN SELECT RAISE(ABORT, 'no'); END;",
        )
        .unwrap();
        assert!(create_workspace(&c, dir.path(), "A", "a@x.com").is_err());
        let left = std::fs::read_dir(dir.path().join("tenants")).map_or(0, |d| d.count());
        assert_eq!(left, 0);
    }
}
