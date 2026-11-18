use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use std::path::{Path, PathBuf};

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

/// Opens (and migrates) `data/control.sqlite`: tenants, users, invites, sessions.
pub fn open_control(data_dir: &Path) -> Result<Connection> {
    let m = Migrations::new(vec![M::up(include_str!("../migrations/control_001.sql"))]);
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
        "INSERT INTO invites (email, tenant_id, role) VALUES (?1, ?2, 'admin')",
        params![email.to_lowercase(), tid],
    )?;
    Ok(())
}

pub struct User {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub tenant_id: String,
    pub role: String,
}

/// Invite-only sign-in: an existing user, or a pending invite which becomes a user.
/// Returns None for anyone else.
pub fn user_for_sign_in(
    control: &Connection,
    email: &str,
    name: Option<&str>,
) -> Result<Option<User>> {
    let email = email.to_lowercase();
    let find = |c: &Connection| -> Result<Option<User>> {
        Ok(c.query_row(
            "SELECT id, email, name, tenant_id, role FROM users WHERE email = ?1",
            [&email],
            |r| {
                Ok(User {
                    id: r.get(0)?,
                    email: r.get(1)?,
                    name: r.get(2)?,
                    tenant_id: r.get(3)?,
                    role: r.get(4)?,
                })
            },
        )
        .optional()?)
    };
    if let Some(u) = find(control)? {
        return Ok(Some(u));
    }
    let invite: Option<(String, String)> = control
        .query_row(
            "SELECT tenant_id, role FROM invites WHERE email = ?1",
            [&email],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((tenant_id, role)) = invite else {
        return Ok(None);
    };
    control.execute(
        "INSERT INTO users (id, email, name, tenant_id, role) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![new_id(), email, name, tenant_id, role],
    )?;
    control.execute("DELETE FROM invites WHERE email = ?1", [&email])?;
    find(control)
}

pub fn create_session(control: &Connection, user_id: &str, ttl_ms: i64) -> Result<String> {
    let id = format!("{}{}", new_id(), new_id()).replace('-', "");
    control.execute(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES (?1, ?2, ?3)",
        params![id, user_id, now_ms() + ttl_ms],
    )?;
    Ok(id)
}

pub fn user_for_session(control: &Connection, session_id: &str) -> Result<Option<User>> {
    Ok(control
        .query_row(
            "SELECT u.id, u.email, u.name, u.tenant_id, u.role FROM sessions s JOIN users u ON u.id = s.user_id
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
}
