use anyhow::Result;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use std::path::{Path, PathBuf};

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
    let m = Migrations::new(vec![M::up(include_str!("../migrations/tenant_001.sql"))]);
    open(&tenant_dir(data_dir, tenant_id).join("recapz.sqlite"), &m)
}

pub fn tenant_ids(control: &Connection) -> Result<Vec<String>> {
    let mut s = control.prepare("SELECT id FROM tenants ORDER BY id")?;
    let ids = s.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(ids)
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
}
