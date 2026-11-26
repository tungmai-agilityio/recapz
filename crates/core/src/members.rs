//! Members of one workspace: list, invite, change role, remove. The people live in the control
//! database (`memberships`, `workspace_invites`, `sessions`), their data in the tenant database
//! (`members` row id = user id). Callers check that the actor is an admin.
use crate::{
    db::{now_ms, INVITE_TTL_MS},
    google_tokens, purge,
};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::{fmt, path::Path};

/// Refusals a route turns into 4xx answers (find them with `downcast_ref`).
#[derive(Debug, PartialEq)]
pub enum Refused {
    InvalidRole,
    AlreadyMember,
    NotMember,
    LastAdmin,
    SelfRemoval,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refused::InvalidRole => "role must be admin or member",
            Refused::AlreadyMember => "already a member of this workspace",
            Refused::NotMember => "not a member of this workspace",
            Refused::LastAdmin => "a workspace needs at least one admin",
            Refused::SelfRemoval => "you cannot remove yourself",
        })
    }
}

impl std::error::Error for Refused {}

#[derive(Debug, PartialEq)]
pub struct Member {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub role: String,
    pub meetings: i64,
}

#[derive(Debug, PartialEq)]
pub struct Invite {
    pub email: String,
    pub role: String,
    pub created_at: i64,
}

fn check_role(role: &str) -> Result<()> {
    match role {
        "admin" | "member" => Ok(()),
        _ => Err(Refused::InvalidRole.into()),
    }
}

/// Members (by email) with the number of meetings they own, then unexpired pending invites.
pub fn list(
    control: &Connection,
    tenant: &Connection,
    tenant_id: &str,
) -> Result<(Vec<Member>, Vec<Invite>)> {
    let mut members: Vec<Member> = control
        .prepare(
            "SELECT u.id, u.email, u.name, m.role FROM memberships m JOIN users u ON u.id = m.user_id
             WHERE m.tenant_id = ?1 ORDER BY u.email",
        )?
        .query_map([tenant_id], |r| {
            Ok(Member {
                id: r.get(0)?,
                email: r.get(1)?,
                name: r.get(2)?,
                role: r.get(3)?,
                meetings: 0,
            })
        })?
        .collect::<Result<_, _>>()?;
    for m in &mut members {
        m.meetings = tenant.query_row(
            "SELECT count(*) FROM meetings WHERE owner_id = ?1",
            [&m.id],
            |r| r.get(0),
        )?;
    }
    let invites = control
        .prepare(
            "SELECT email, role, created_at FROM workspace_invites WHERE tenant_id = ?1 AND created_at > ?2
             ORDER BY email",
        )?
        .query_map(params![tenant_id, now_ms() - INVITE_TTL_MS], |r| {
            Ok(Invite {
                email: r.get(0)?,
                role: r.get(1)?,
                created_at: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok((members, invites))
}

/// Creates (or renews, with the new role) a 14-day invite. Refuses an email that is already a member.
pub fn invite(
    control: &Connection,
    tenant_id: &str,
    invited_by: &str,
    email: &str,
    role: &str,
) -> Result<()> {
    check_role(role)?;
    let email = email.trim().to_lowercase();
    let member: bool = control.query_row(
        "SELECT EXISTS (SELECT 1 FROM memberships m JOIN users u ON u.id = m.user_id
                        WHERE m.tenant_id = ?1 AND u.email = ?2)",
        params![tenant_id, email],
        |r| r.get(0),
    )?;
    if member {
        return Err(Refused::AlreadyMember.into());
    }
    control.execute(
        "INSERT INTO workspace_invites (email, tenant_id, role, invited_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (email, tenant_id) DO UPDATE SET role = ?3, invited_by = ?4, created_at = ?5",
        params![email, tenant_id, role, invited_by, now_ms()],
    )?;
    Ok(())
}

/// False when there was no such invite.
pub fn cancel_invite(control: &Connection, tenant_id: &str, email: &str) -> Result<bool> {
    Ok(control.execute(
        "DELETE FROM workspace_invites WHERE tenant_id = ?1 AND email = ?2",
        params![tenant_id, email.trim().to_lowercase()],
    )? > 0)
}

/// Fails with `LastAdmin` when `user_id` is the only admin; the caller runs it inside its transaction.
fn keep_an_admin(control: &Connection, tenant_id: &str, user_id: &str) -> Result<()> {
    let others: i64 = control.query_row(
        "SELECT count(*) FROM memberships WHERE tenant_id = ?1 AND role = 'admin' AND user_id != ?2",
        params![tenant_id, user_id],
        |r| r.get(0),
    )?;
    if others == 0 {
        return Err(Refused::LastAdmin.into());
    }
    Ok(())
}

fn role_of(control: &Connection, tenant_id: &str, user_id: &str) -> Result<String> {
    control
        .query_row(
            "SELECT role FROM memberships WHERE tenant_id = ?1 AND user_id = ?2",
            params![tenant_id, user_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Refused::NotMember.into())
}

/// Runs `f` in one transaction on `c`.
fn in_tx<T>(c: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    c.execute_batch("BEGIN IMMEDIATE")?;
    let done = f();
    c.execute_batch(if done.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    done
}

/// Changes a member's role; demoting the last admin is refused. The tenant `members` row follows on
/// the person's next request (`ensure_member`).
pub fn set_role(control: &Connection, tenant_id: &str, user_id: &str, role: &str) -> Result<()> {
    check_role(role)?;
    in_tx(control, || {
        if role_of(control, tenant_id, user_id)? == "admin" && role != "admin" {
            keep_an_admin(control, tenant_id, user_id)?;
        }
        control.execute(
            "UPDATE memberships SET role = ?3 WHERE tenant_id = ?1 AND user_id = ?2",
            params![tenant_id, user_id, role],
        )?;
        Ok(())
    })
}

/// Removes a member (never yourself, never the last admin): the tenant side first (documents and
/// calendar connection hard-deleted, upcoming bookings dropped, meetings handed to `admin_id` with
/// their `share`, `members.status = 'removed'`), then the control side (sessions in this workspace
/// and the membership). The order lets a failed call be repeated; the guards run before anything
/// is deleted and again inside the control transaction.
pub fn remove(
    control: &Connection,
    tenant: &Connection,
    data_dir: &Path,
    tenant_id: &str,
    admin_id: &str,
    user_id: &str,
) -> Result<()> {
    if user_id == admin_id {
        return Err(Refused::SelfRemoval.into());
    }
    if role_of(control, tenant_id, user_id)? == "admin" {
        keep_an_admin(control, tenant_id, user_id)?;
    }
    let docs: Vec<String> = tenant
        .prepare("SELECT id FROM documents WHERE member_id = ?1")?
        .query_map([user_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for d in docs {
        purge::purge_document(tenant, data_dir, tenant_id, user_id, &d)?;
    }
    in_tx(tenant, || {
        tenant.execute(
            "UPDATE meetings SET owner_id = ?1 WHERE owner_id = ?2",
            params![admin_id, user_id],
        )?;
        google_tokens::delete(tenant, user_id)?;
        tenant.execute(
            "DELETE FROM calendar_events WHERE member_id = ?1 AND ends_at > ?2",
            params![user_id, now_ms()],
        )?;
        tenant.execute(
            "UPDATE members SET status = 'removed' WHERE id = ?1",
            [user_id],
        )?;
        Ok(())
    })?;
    in_tx(control, || {
        if role_of(control, tenant_id, user_id)? == "admin" {
            keep_an_admin(control, tenant_id, user_id)?;
        }
        control.execute(
            "DELETE FROM sessions WHERE user_id = ?1
               AND COALESCE(tenant_id, (SELECT tenant_id FROM users WHERE id = ?1)) = ?2",
            params![user_id, tenant_id],
        )?;
        control.execute(
            "DELETE FROM memberships WHERE user_id = ?1 AND tenant_id = ?2",
            params![user_id, tenant_id],
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{create_session, open_control, open_tenant, tenant_dir};

    fn n(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn refused(r: Result<impl std::fmt::Debug>) -> Refused {
        match r.unwrap_err().downcast::<Refused>() {
            Ok(e) => e,
            Err(e) => panic!("not a refusal: {e}"),
        }
    }

    /// t1: admin a (owns m0), member b (owns m1 shared 'all', m2); t2: b is also a member and owns m9.
    fn setup(dir: &Path) -> (Connection, Connection, Connection) {
        let control = open_control(dir).unwrap();
        control
            .execute_batch(
                "INSERT INTO tenants (id, name) VALUES ('t1','One'), ('t2','Two');
                 INSERT INTO users (id, email, name, tenant_id, role) VALUES
                   ('a','a@x.io','A','t1','admin'), ('b','b@x.io','B','t1','member');
                 INSERT INTO memberships (user_id, tenant_id, role, created_at) VALUES
                   ('a','t1','admin',1), ('b','t1','member',2), ('b','t2','member',3);",
            )
            .unwrap();
        let t1 = open_tenant(dir, "t1").unwrap();
        t1.execute_batch(
            "INSERT INTO members (id, email, name, role) VALUES ('a','a@x.io','A','admin'), ('b','b@x.io','B','member');
             INSERT INTO meetings (id, title, platform, status, owner_id, share) VALUES
               ('m0','zero','upload','ready','a','attendees'),
               ('m1','one','upload','ready','b','all'),
               ('m2','two','upload','ready','b','private');
             INSERT INTO documents (id, member_id, name, kind, size_bytes, path, created_at)
               VALUES ('d1','b','n.md','md',1,'docs/d1.md',1);
             INSERT INTO calendar_events (id, member_id, external_id, title, starts_at, ends_at)
               VALUES ('e-past','b','x1','past',1,2), ('e-next','b','x2','next',9000000000000,9000000001000);
             INSERT INTO google_tokens (member_id, refresh_token, nonce, scopes, created_at, updated_at)
               VALUES ('b', x'01', x'02', 's', 1, 1);",
        )
        .unwrap();
        std::fs::create_dir_all(tenant_dir(dir, "t1").join("docs")).unwrap();
        std::fs::write(tenant_dir(dir, "t1").join("docs/d1.md"), "secret").unwrap();
        let t2 = open_tenant(dir, "t2").unwrap();
        t2.execute_batch(
            "INSERT INTO members (id, email, name, role) VALUES ('b','b@x.io','B','member');
             INSERT INTO meetings (id, title, platform, status, owner_id) VALUES ('m9','nine','upload','ready','b');",
        )
        .unwrap();
        (control, t1, t2)
    }

    #[test]
    fn lists_members_with_meeting_counts_and_unexpired_invites() {
        let dir = tempfile::tempdir().unwrap();
        let (control, t1, _t2) = setup(dir.path());
        invite(&control, "t1", "a", "New@X.io ", "member").unwrap();
        control
            .execute(
                "INSERT INTO workspace_invites (email, tenant_id, role, created_at) VALUES ('old@x.io','t1','member',1)",
                [],
            )
            .unwrap();
        let (members, invites) = list(&control, &t1, "t1").unwrap();
        let got: Vec<_> = members
            .iter()
            .map(|m| (m.email.as_str(), m.role.as_str(), m.meetings))
            .collect();
        assert_eq!(got, [("a@x.io", "admin", 1), ("b@x.io", "member", 2)]);
        let got: Vec<_> = invites
            .iter()
            .map(|i| (i.email.as_str(), i.role.as_str()))
            .collect();
        assert_eq!(got, [("new@x.io", "member")]);
    }

    #[test]
    fn invite_refuses_a_member_and_renews_a_pending_invite() {
        let dir = tempfile::tempdir().unwrap();
        let (control, _t1, _t2) = setup(dir.path());
        assert_eq!(
            refused(invite(&control, "t1", "a", "B@x.io", "member")),
            Refused::AlreadyMember
        );
        assert_eq!(
            refused(invite(&control, "t1", "a", "c@x.io", "owner")),
            Refused::InvalidRole
        );
        invite(&control, "t1", "a", "c@x.io", "member").unwrap();
        invite(&control, "t1", "a", "c@x.io", "admin").unwrap();
        assert_eq!(
            n(
                &control,
                "SELECT count(*) FROM workspace_invites WHERE role = 'admin' AND email = 'c@x.io'"
            ),
            1
        );
        assert!(cancel_invite(&control, "t1", "C@x.io").unwrap());
        assert!(!cancel_invite(&control, "t1", "c@x.io").unwrap());
    }

    #[test]
    fn the_last_admin_cannot_be_demoted_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let (control, t1, _t2) = setup(dir.path());
        assert_eq!(
            refused(set_role(&control, "t1", "a", "member")),
            Refused::LastAdmin
        );
        // b is not an admin of t1, so an admin removing the only admin is the guard, not self-removal.
        assert_eq!(
            refused(remove(&control, &t1, dir.path(), "t1", "b", "a")),
            Refused::LastAdmin
        );
        assert_eq!(
            n(
                &control,
                "SELECT count(*) FROM memberships WHERE tenant_id = 't1' AND role = 'admin'"
            ),
            1
        );

        set_role(&control, "t1", "b", "admin").unwrap();
        set_role(&control, "t1", "a", "member").unwrap();
        assert_eq!(
            refused(set_role(&control, "t1", "b", "member")),
            Refused::LastAdmin
        );
    }

    #[test]
    fn nobody_removes_themselves() {
        let dir = tempfile::tempdir().unwrap();
        let (control, t1, _t2) = setup(dir.path());
        set_role(&control, "t1", "b", "admin").unwrap();
        assert_eq!(
            refused(remove(&control, &t1, dir.path(), "t1", "b", "b")),
            Refused::SelfRemoval
        );
        assert_eq!(
            n(
                &control,
                "SELECT count(*) FROM memberships WHERE tenant_id = 't1'"
            ),
            2
        );
    }

    #[test]
    fn remove_leaves_exactly_the_expected_rows_in_both_databases() {
        let dir = tempfile::tempdir().unwrap();
        let (control, t1, t2) = setup(dir.path());
        let sess_t1 = create_session(&control, "b", 1_000_000).unwrap();
        control
            .execute(
                "UPDATE sessions SET tenant_id = 't1' WHERE id = ?1",
                [&sess_t1],
            )
            .unwrap();
        let sess_t2 = create_session(&control, "b", 1_000_000).unwrap();
        control
            .execute(
                "UPDATE sessions SET tenant_id = 't2' WHERE id = ?1",
                [&sess_t2],
            )
            .unwrap();

        remove(&control, &t1, dir.path(), "t1", "a", "b").unwrap();

        // control: membership and the t1 session gone; the t2 membership, session and the user stay
        assert_eq!(
            n(
                &control,
                "SELECT count(*) FROM memberships WHERE user_id = 'b' AND tenant_id = 't1'"
            ),
            0
        );
        assert_eq!(
            n(
                &control,
                "SELECT count(*) FROM memberships WHERE user_id = 'b' AND tenant_id = 't2'"
            ),
            1
        );
        assert_eq!(n(&control, "SELECT count(*) FROM memberships"), 2);
        assert_eq!(
            n(
                &control,
                "SELECT count(*) FROM sessions WHERE tenant_id = 't1'"
            ),
            0
        );
        assert_eq!(
            n(
                &control,
                "SELECT count(*) FROM sessions WHERE tenant_id = 't2'"
            ),
            1
        );
        assert_eq!(n(&control, "SELECT count(*) FROM users"), 2);
        // tenant: meetings moved with share kept, documents/calendar gone, row kept as removed
        let owners: Vec<(String, String, String)> = t1
            .prepare("SELECT id, owner_id, share FROM meetings ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let want = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
        assert_eq!(
            owners,
            [
                want("m0", "a", "attendees"),
                want("m1", "a", "all"),
                want("m2", "a", "private")
            ]
        );
        assert_eq!(n(&t1, "SELECT count(*) FROM documents"), 0);
        assert!(!tenant_dir(dir.path(), "t1").join("docs/d1.md").exists());
        assert_eq!(n(&t1, "SELECT count(*) FROM google_tokens"), 0);
        let events: Vec<String> = t1
            .prepare("SELECT id FROM calendar_events")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(events, ["e-past"]);
        let status: String = t1
            .query_row("SELECT status FROM members WHERE id = 'b'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "removed");
        assert_eq!(
            n(&t1, "SELECT count(*) FROM members WHERE status = 'active'"),
            1
        );
        // the other workspace is untouched
        assert_eq!(
            n(&t2, "SELECT count(*) FROM meetings WHERE owner_id = 'b'"),
            1
        );
        assert_eq!(
            n(&t2, "SELECT count(*) FROM members WHERE status = 'active'"),
            1
        );

        // they can be invited again
        invite(&control, "t1", "a", "b@x.io", "member").unwrap();
    }
}
