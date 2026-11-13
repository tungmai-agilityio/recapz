//! Starts the notetaker for real: every 15 s it books calendar events that are about to start,
//! claims `scheduled` meetings (capped), and runs each call in its own task with a Meet driver.
//! The rules for one call (what ends it, what status it leaves) are in `notetaker`.
use crate::notetaker::{claim_scheduled, fail, recover, run_meeting};
use anyhow::Result;
use recapz_core::{
    calendar::book_due,
    db::{self, now_ms},
};
use recapz_notetaker::{
    browser::Launch,
    meet::{MeetConfig, MeetDriver},
    profile,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const TICK: Duration = Duration::from_secs(15);
/// A calendar event is booked this long before it starts, so the notetaker is in the lobby on time.
const LEAD_MS: i64 = 60_000;

/// What the call posts in chat on admission. A setting for it comes with the settings API; the
/// words "opt out" must stay, they are what members are told to type.
const NOTICE: &str = "Recapz notetaker is recording and transcribing this call. \
Type 'opt out' to make it leave and delete the recording.";

pub struct Settings {
    pub data_dir: PathBuf,
    /// `notetaker.chromium_path`, already checked to exist.
    pub chromium: PathBuf,
    pub max_concurrent: usize,
    pub max_call_minutes: u32,
}

fn tmp_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("notetaker-tmp")
}

/// Runs once at startup, before any call: calls cut off by a restart are failed or transcribed, and
/// browser copies left by a killed server are deleted (no call is running yet, so none is in use).
pub fn startup(data_dir: &Path) -> Result<()> {
    let control = db::open_control(data_dir)?;
    for tenant in db::tenant_ids(&control)? {
        recover(&db::open_tenant(data_dir, &tenant)?, data_dir, &tenant)?;
    }
    match fs::remove_dir_all(tmp_dir(data_dir)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// One pass over every tenant: books due events, then claims scheduled meetings up to the cap.
/// Returns `(tenant, meeting id, join url)` for each call to start.
pub fn due_calls(
    data_dir: &Path,
    max_concurrent: usize,
    now: i64,
) -> Result<Vec<(String, String, String)>> {
    let control = db::open_control(data_dir)?;
    let mut out = Vec::new();
    for tenant in db::tenant_ids(&control)? {
        let c = db::open_tenant(data_dir, &tenant)?;
        book_due(&c, now, LEAD_MS)?;
        // The cap is across tenants: what this pass already claimed counts as active.
        let room = max_concurrent.saturating_sub(out.len());
        for (id, url) in claim_scheduled(&c, room)? {
            out.push((tenant.clone(), id, url));
        }
    }
    Ok(out)
}

/// Why a call cannot start at all, as the error shown on the meeting.
pub fn cannot_start(data_dir: &Path, display: bool) -> Option<&'static str> {
    if !profile::has_login(data_dir) {
        Some("notetaker_signed_out")
    } else if !display {
        Some("notetaker_no_display")
    } else {
        None
    }
}

async fn call(s: Arc<Settings>, tenant: String, id: String, url: String) {
    let display = profile::display_available(
        std::env::var("DISPLAY").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        cfg!(target_os = "linux"),
    );
    let (d, t, i) = (s.data_dir.clone(), tenant.clone(), id.clone());
    let result = async {
        let c = db::open_tenant(&s.data_dir, &tenant)?;
        if let Some(why) = cannot_start(&s.data_dir, display) {
            return fail(&c, &s.data_dir, &tenant, &id, why);
        }
        let mut driver = MeetDriver::new(MeetConfig {
            launch: Launch {
                chromium: Some(s.chromium.clone()),
                profile: Some(profile::profile_dir(&s.data_dir)),
                work_dir: tmp_dir(&s.data_dir).join(&id),
                headless: false,
            },
            notice: NOTICE.to_string(),
        });
        let max_call_ms = i64::from(s.max_call_minutes.max(1)) * 60_000;
        run_meeting(
            c,
            &s.data_dir,
            &tenant,
            &id,
            &url,
            max_call_ms,
            &mut driver,
            now_ms,
        )
        .await
    }
    .await;
    if let Err(e) = result {
        // The meeting id only: never the join link or anything said in the call.
        tracing::error!("notetaker call {i} failed: {e:#}");
        mark_error(&d, &t, &i);
    }
}

/// Each call gets its own thread and runtime: the call's database handle cannot cross threads while
/// it waits on the browser, and a stuck call then cannot stall the server's runtime.
fn start(s: Arc<Settings>, tenant: String, id: String, url: String) {
    let spawned = std::thread::Builder::new()
        .name("notetaker-call".into())
        .spawn({
            let (s, tenant, id) = (s.clone(), tenant.clone(), id.clone());
            move || match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt.block_on(call(s, tenant, id, url)),
                Err(e) => {
                    tracing::error!("notetaker runtime: {e}");
                    mark_error(&s.data_dir, &tenant, &id);
                }
            }
        });
    if let Err(e) = spawned {
        tracing::error!("could not start the call thread: {e}");
        mark_error(&s.data_dir, &tenant, &id);
    }
}

fn mark_error(data_dir: &Path, tenant: &str, id: &str) {
    let _ = db::open_tenant(data_dir, tenant)
        .and_then(|c| fail(&c, data_dir, tenant, id, "notetaker_error"));
}

/// Starts the loop. Call `startup` first.
pub fn spawn(s: Settings) {
    let s = Arc::new(s);
    tokio::spawn(async move {
        loop {
            let (d, n) = (s.data_dir.clone(), s.max_concurrent);
            match tokio::task::spawn_blocking(move || due_calls(&d, n, now_ms())).await {
                Ok(Ok(calls)) => {
                    for (tenant, id, url) in calls {
                        start(s.clone(), tenant, id, url);
                    }
                }
                Ok(Err(e)) => tracing::error!("notetaker loop: {e:#}"),
                Err(e) => tracing::error!("notetaker loop panicked: {e}"),
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let control = db::open_control(dir.path()).unwrap();
        control
            .execute("INSERT INTO tenants (id, name) VALUES ('t1', 'Team')", [])
            .unwrap();
        let c = db::open_tenant(dir.path(), "t1").unwrap();
        (dir, c)
    }

    fn meeting(c: &rusqlite::Connection, id: &str, status: &str, created: i64) {
        c.execute(
            "INSERT INTO meetings (id, title, platform, status, join_url, created_at) VALUES (?1, 't', 'meet', ?2, 'https://meet.example.test/abc-defg-hij', ?3)",
            rusqlite::params![id, status, created],
        )
        .unwrap();
    }

    fn status(c: &rusqlite::Connection, id: &str) -> String {
        c.query_row("SELECT status FROM meetings WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn a_pass_claims_scheduled_meetings_up_to_the_cap_counting_active_calls() {
        let (dir, c) = setup();
        meeting(&c, "live", "recording", 1);
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            meeting(&c, id, "scheduled", 10 + i as i64);
        }
        let got = due_calls(dir.path(), 2, 1_000).unwrap();
        assert_eq!(
            got.iter()
                .map(|(t, i, _)| (t.as_str(), i.as_str()))
                .collect::<Vec<_>>(),
            vec![("t1", "a")],
            "one call is already active, so only one more fits"
        );
        assert_eq!(
            (status(&c, "a"), status(&c, "b")),
            ("joining".into(), "scheduled".into())
        );
        assert!(
            due_calls(dir.path(), 2, 2_000).unwrap().is_empty(),
            "now full"
        );
    }

    #[test]
    fn startup_fails_a_joining_call_and_sweeps_leftover_browser_copies() {
        let (dir, c) = setup();
        meeting(&c, "j", "joining", 1);
        let left = tmp_dir(dir.path()).join("old-call");
        fs::create_dir_all(&left).unwrap();
        fs::write(left.join("Cookies"), "c").unwrap();
        startup(dir.path()).unwrap();
        assert_eq!(status(&c, "j"), "failed");
        let why: String = c
            .query_row("SELECT error FROM meetings WHERE id='j'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(why, "server_restarted");
        assert!(!tmp_dir(dir.path()).exists());
        startup(dir.path()).unwrap(); // nothing to sweep is fine
    }

    #[test]
    fn a_call_cannot_start_without_a_saved_sign_in_or_a_display() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(cannot_start(dir.path(), true), Some("notetaker_signed_out"));
        fs::create_dir_all(profile::profile_dir(dir.path()).join("Default")).unwrap();
        assert_eq!(
            cannot_start(dir.path(), false),
            Some("notetaker_no_display")
        );
        assert_eq!(cannot_start(dir.path(), true), None);
    }

    #[test]
    fn a_switched_on_event_starting_within_the_lead_is_booked_and_claimed_in_one_pass() {
        let (dir, c) = setup();
        c.execute(
            "INSERT INTO members (id, email, role) VALUES ('u1','a@example.test','member')",
            [],
        )
        .unwrap();
        for (id, start) in [("soon", 1_030_000), ("later", 1_200_000)] {
            c.execute(
                "INSERT INTO calendar_events (id, member_id, external_id, title, starts_at, ends_at, join_url, platform, auto_join)
                 VALUES (?1, 'u1', ?1, ?1, ?2, ?2 + 3600000, 'https://meet.example.test/abc-defg-hij', 'meet', 1)",
                rusqlite::params![id, start],
            )
            .unwrap();
        }
        let got = due_calls(dir.path(), 2, 1_000_000).unwrap();
        assert_eq!(got.len(), 1, "only the event starting within a minute");
        assert_eq!(got[0].2, "https://meet.example.test/abc-defg-hij");
        assert_eq!(status(&c, &got[0].1), "joining");
    }

    #[test]
    fn the_notice_names_the_phrase_people_type_to_opt_out() {
        assert!(NOTICE.contains("'opt out'"));
        assert!(recapz_notetaker::optout::is_opt_out("opt out"));
    }
}
