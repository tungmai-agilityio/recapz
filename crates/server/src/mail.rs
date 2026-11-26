//! Emails the meeting report link to attendees who are members, once per member and meeting, and
//! the sign-in link to a person an admin invites.
//! The transport sits behind `Mailer`, so tests need no SMTP server. Logs hold meeting IDs only.
use crate::config::Mail;
use anyhow::{Context, Result};
use lettre::{
    message::Mailbox, transport::smtp::authentication::Credentials, Message, SmtpTransport,
    Transport,
};
use recapz_core::db::MEETING_VISIBLE;
use rusqlite::{params, Connection};

pub trait Mailer: Send + Sync {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<()>;
}

pub struct SmtpMailer {
    from: Mailbox,
    smtp: SmtpTransport,
}

impl SmtpMailer {
    /// `None` when no relay `host` is configured (the email is off).
    pub fn from_config(c: &Mail) -> Result<Option<Self>> {
        if c.host.trim().is_empty() {
            return Ok(None);
        }
        let from = c.from.parse().context("[mail] from is not an address")?;
        // Port 465 is implicit TLS; anything else upgrades with STARTTLS (never plaintext).
        let b = if c.port == 465 {
            SmtpTransport::relay(&c.host)?
        } else {
            SmtpTransport::starttls_relay(&c.host)?
        }
        .port(c.port);
        let b = if c.username.is_empty() {
            b
        } else {
            b.credentials(Credentials::new(c.username.clone(), c.password.clone()))
        };
        Ok(Some(Self {
            from,
            smtp: b.build(),
        }))
    }
}

impl Mailer for SmtpMailer {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        let m = Message::builder()
            .from(self.from.clone())
            .to(to.parse()?)
            .subject(subject)
            .body(body.to_string())?;
        self.smtp.send(&m)?;
        Ok(())
    }
}

/// Subject and body of the invite email: the workspace, the address to sign in as, and the link.
pub fn invite_email(workspace: &str, to: &str, link: &str) -> (String, String) {
    (
        format!("You're invited to {workspace} on Recapz"),
        format!(
            "You've been invited to the {workspace} workspace on Recapz.\n\n\
             Sign in with Google as {to} to join:\n{link}\n"
        ),
    )
}

/// Sends the report link to every active member on the invite who may see the meeting. A member
/// already mailed for this meeting (marker in `settings`) is skipped, so a retried or regenerated
/// report never mails twice. The marker is claimed before sending and released if the send fails.
/// Returns how many were sent. ponytail: a crash between claim and send loses that one email.
pub fn send_report_ready(
    c: &Connection,
    mailer: &dyn Mailer,
    base_url: &str,
    meeting_id: &str,
) -> Result<usize> {
    let title: String = c.query_row(
        "SELECT title FROM meetings WHERE id = ?1",
        [meeting_id],
        |r| r.get(0),
    )?;
    let sql = format!(
        "SELECT mb.id, mb.email FROM members mb
         WHERE mb.status = 'active'
           AND EXISTS (SELECT 1 FROM calendar_events e, json_each(e.attendees) a
                       WHERE e.meeting_id = ?1
                         AND lower(json_extract(a.value, '$.email')) = lower(mb.email))
           AND EXISTS (SELECT 1 FROM meetings m WHERE m.id = ?1 AND {})
         ORDER BY mb.id",
        MEETING_VISIBLE.replace("?1", "mb.id")
    );
    let people: Vec<(String, String)> = c
        .prepare(&sql)?
        .query_map(params![meeting_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let link = format!("{}/meetings/{meeting_id}", base_url.trim_end_matches('/'));
    let mut sent = 0;
    for (member, email) in people {
        let key = format!("report_mailed:{meeting_id}:{member}");
        let claimed = c.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, '1')",
            [&key],
        )?;
        if claimed == 0 {
            continue;
        }
        let sent_ok = mailer.send(
            &email,
            &format!("Meeting report ready: {title}"),
            &format!("The report for \"{title}\" is ready.\n\n{link}\n"),
        );
        if sent_ok.is_ok() {
            sent += 1;
        } else {
            c.execute("DELETE FROM settings WHERE key = ?1", [&key])?;
            // The error text can name the address, so only the meeting ID is logged.
            tracing::warn!("report email failed for meeting {meeting_id}");
        }
    }
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use recapz_core::db::open_tenant;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        sent: Mutex<Vec<(String, String, String)>>,
        fail: Mutex<bool>,
    }
    impl Mailer for Fake {
        fn send(&self, to: &str, subject: &str, body: &str) -> Result<()> {
            if *self.fail.lock().unwrap() {
                anyhow::bail!("relay down");
            }
            self.sent
                .lock()
                .unwrap()
                .push((to.into(), subject.into(), body.into()));
            Ok(())
        }
    }

    fn setup() -> Connection {
        let d = tempfile::tempdir().unwrap();
        let c = open_tenant(d.path(), "t1").unwrap();
        std::mem::forget(d);
        c.execute_batch(
            "INSERT INTO members (id,email,role,status) VALUES
               ('a','ann@x.io','admin','active'), ('b','Bob@x.io','member','active'),
               ('c','cy@x.io','member','removed'), ('d','dee@x.io','member','active');
             INSERT INTO meetings (id,title,platform,status,owner_id,share)
               VALUES ('m1','Plan','meet','ready','a','attendees');
             INSERT INTO calendar_events (id,member_id,external_id,title,starts_at,ends_at,meeting_id,attendees)
               VALUES ('e1','a','x','Plan',0,1,'m1',
               '[{\"email\":\"bob@x.io\"},{\"email\":\"cy@x.io\"},{\"email\":\"out@y.io\"}]');",
        )
        .unwrap();
        c
    }

    #[test]
    fn mails_only_active_member_attendees_with_a_link_and_never_twice() {
        let c = setup();
        let f = Fake::default();
        assert_eq!(send_report_ready(&c, &f, "https://r.io/", "m1").unwrap(), 1);
        {
            let s = f.sent.lock().unwrap();
            assert_eq!(
                s.len(),
                1,
                "not the removed member, the non-member or non-attendees"
            );
            assert_eq!(s[0].0, "Bob@x.io");
            assert_eq!(s[0].1, "Meeting report ready: Plan");
            assert!(s[0].2.contains("https://r.io/meetings/m1"), "{}", s[0].2);
        }
        assert_eq!(send_report_ready(&c, &f, "https://r.io/", "m1").unwrap(), 0);
        assert_eq!(f.sent.lock().unwrap().len(), 1, "a retry sends nothing");
    }

    #[test]
    fn a_failed_send_is_retried_and_an_owner_only_meeting_mails_nobody() {
        let c = setup();
        let f = Fake::default();
        *f.fail.lock().unwrap() = true;
        assert_eq!(send_report_ready(&c, &f, "https://r.io", "m1").unwrap(), 0);
        *f.fail.lock().unwrap() = false;
        assert_eq!(send_report_ready(&c, &f, "https://r.io", "m1").unwrap(), 1);
        c.execute("UPDATE meetings SET share='owner'", []).unwrap();
        c.execute("DELETE FROM settings", []).unwrap();
        assert_eq!(send_report_ready(&c, &f, "https://r.io", "m1").unwrap(), 0);
    }
}
