//! Starts the notetaker for real: every 15 s it books calendar events that are about to start,
//! claims `scheduled` meetings (capped), and runs each call in its own task with a Meet driver.
//! The rules for one call (what ends it, what status it leaves) are in `notetaker`.
use crate::notetaker::{claim_scheduled, fail, recover, run_meeting, salvage};
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

/// One pass over every tenant: books due events, then claims scheduled meetings so that at most
/// `max_concurrent` calls are active across all tenants. A tenant that errors is logged and skipped,
/// so it neither blocks the others nor loses what they already claimed.
/// Returns `(tenant, meeting id, join url)` for each call to start.
pub fn due_calls(
    data_dir: &Path,
    max_concurrent: usize,
    now: i64,
) -> Result<Vec<(String, String, String)>> {
    let control = db::open_control(data_dir)?;
    let mut tenants = Vec::new();
    let mut active_total = 0usize;
    for tenant in db::tenant_ids(&control)? {
        match open_and_count(data_dir, &tenant, now) {
            Ok((c, active)) => {
                active_total += active;
                tenants.push((tenant, c, active));
            }
            Err(e) => tracing::error!("notetaker loop, tenant {tenant}: {e:#}"),
        }
    }
    let mut out = Vec::new();
    for (tenant, c, active) in tenants {
        let free = max_concurrent.saturating_sub(active_total + out.len());
        // `claim_scheduled` caps on this tenant's own active calls, so give it room on top of them.
        match claim_scheduled(&c, active + free) {
            Ok(claimed) => out.extend(
                claimed
                    .into_iter()
                    .map(|(id, url)| (tenant.clone(), id, url)),
            ),
            Err(e) => tracing::error!("notetaker loop, tenant {tenant}: {e:#}"),
        }
    }
    Ok(out)
}

fn open_and_count(
    data_dir: &Path,
    tenant: &str,
    now: i64,
) -> Result<(rusqlite::Connection, usize)> {
    let c = db::open_tenant(data_dir, tenant)?;
    book_due(&c, now, LEAD_MS)?;
    let active = c.query_row(
        "SELECT COUNT(*) FROM meetings WHERE status IN ('joining','recording')",
        [],
        |r| r.get(0),
    )?;
    Ok((c, active))
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
        salvage_logged(&d, &t, &i);
    }
}

/// Each call gets its own thread and runtime: the call's database handle cannot cross threads while
/// it waits on the browser, and a stuck call then cannot stall the server's runtime.
fn start(s: Arc<Settings>, tenant: String, id: String, url: String) {
    let spawned = std::thread::Builder::new()
        .name("notetaker-call".into())
        .spawn({
            let (s, tenant, id) = (s.clone(), tenant.clone(), id.clone());
            move || {
                guarded(&s.data_dir, &tenant, &id, || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("a current-thread runtime")
                        .block_on(call(s.clone(), tenant.clone(), id.clone(), url));
                })
            }
        });
    if let Err(e) = spawned {
        tracing::error!("could not start the call thread: {e}");
        salvage_logged(&s.data_dir, &tenant, &id);
    }
}

/// Runs a call; a panic anywhere in it (driver, capture, database) settles the meeting instead of
/// leaving it `joining` or `recording` and counting against the cap until the next restart.
fn guarded(data_dir: &Path, tenant: &str, id: &str, run: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).is_err() {
        tracing::error!("notetaker call {id} panicked");
        salvage_logged(data_dir, tenant, id);
    }
}

/// A call that ended in an error or a panic: keep what was recorded, fail what was not. Logged when
/// even that fails, because the meeting then stays `joining` or `recording` until the next restart.
fn salvage_logged(data_dir: &Path, tenant: &str, id: &str) {
    let done = db::open_tenant(data_dir, tenant).and_then(|c| {
        salvage(
            &c,
            data_dir,
            tenant,
            id,
            "The recording may be cut short because the call ended with an error",
            "notetaker_error",
        )
    });
    if let Err(e) = done {
        tracing::error!("notetaker call {id}: could not settle the meeting: {e:#}");
    }
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
    fn the_cap_holds_across_tenants_and_earlier_ticks() {
        let (dir, c1) = setup();
        let control = db::open_control(dir.path()).unwrap();
        control
            .execute("INSERT INTO tenants (id, name) VALUES ('t2', 'Other')", [])
            .unwrap();
        let c2 = db::open_tenant(dir.path(), "t2").unwrap();
        meeting(&c1, "r1", "recording", 1);
        meeting(&c1, "r2", "joining", 2);
        meeting(&c2, "s1", "scheduled", 3);
        assert!(
            due_calls(dir.path(), 2, 1_000).unwrap().is_empty(),
            "two calls are active in the other tenant"
        );
        assert_eq!(status(&c2, "s1"), "scheduled");
        // Three slots, two taken: one more across both tenants, wherever it is waiting.
        meeting(&c1, "s2", "scheduled", 4);
        meeting(&c2, "s3", "scheduled", 5);
        assert_eq!(due_calls(dir.path(), 3, 1_000).unwrap().len(), 1);
    }

    #[test]
    fn a_broken_tenant_does_not_block_the_others_or_lose_their_claims() {
        let (dir, c) = setup();
        let control = db::open_control(dir.path()).unwrap();
        control
            .execute("INSERT INTO tenants (id, name) VALUES ('t0', 'Broken')", [])
            .unwrap();
        control
            .execute(
                "INSERT INTO tenants (id, name) VALUES ('t9', 'Broken too')",
                [],
            )
            .unwrap();
        for t in ["t0", "t9"] {
            let broken = db::tenant_dir(dir.path(), t);
            fs::create_dir_all(&broken).unwrap();
            fs::write(broken.join("recapz.sqlite"), "not a database").unwrap();
        }
        meeting(&c, "ok", "scheduled", 1);
        let got = due_calls(dir.path(), 2, 1_000).unwrap();
        assert_eq!(
            got.len(),
            1,
            "t1's meeting is claimed whichever side the broken ones sort"
        );
        assert_eq!(status(&c, "ok"), "joining");
    }

    #[test]
    fn a_panicking_call_is_settled_and_a_recording_keeps_its_audio() {
        let (dir, c) = setup();
        let audio = tenant_dir_audio(dir.path(), "rec");
        fs::create_dir_all(&audio).unwrap();
        let mut w = recapz_notetaker::wav::WavWriter::new(std::io::BufWriter::new(
            fs::File::create(audio.join("recording.wav")).unwrap(),
        ))
        .unwrap();
        w.write(&vec![0i16; 16_000]).unwrap();
        drop(w); // a header that was never finished, like a cut-off call
        meeting(&c, "rec", "recording", 1);
        c.execute(
            "UPDATE meetings SET audio_path='audio/rec/recording.wav' WHERE id='rec'",
            [],
        )
        .unwrap();
        meeting(&c, "join", "joining", 2);
        meeting(&c, "other", "recording", 3); // another live call: must not be touched
        guarded(dir.path(), "t1", "rec", || panic!("driver bug"));
        guarded(dir.path(), "t1", "join", || panic!("driver bug"));
        assert_eq!(status(&c, "rec"), "transcribing");
        assert!(
            audio.join("recording.wav").exists(),
            "the recording is kept"
        );
        assert_eq!(status(&c, "join"), "failed");
        let why: String = c
            .query_row("SELECT error FROM meetings WHERE id='join'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(why, "notetaker_error");
        assert_eq!(status(&c, "other"), "recording");
        // A call that already finished is left as it is.
        guarded(dir.path(), "t1", "rec", || panic!("late"));
        assert_eq!(status(&c, "rec"), "transcribing");
    }

    fn tenant_dir_audio(data: &Path, id: &str) -> PathBuf {
        db::tenant_dir(data, "t1").join("audio").join(id)
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
