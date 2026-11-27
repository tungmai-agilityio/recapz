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

/// The chat notice: the saved message with `{name}` filled in. The words "opt out" are appended
/// when the message lacks them, so consent is never lost.
pub fn build_notice(message: &str, name: &str) -> String {
    let mut n = message.replace("{name}", name);
    if !n.to_lowercase().contains("opt out") {
        n.push_str(" Type 'opt out' to make it leave and delete the recording.");
    }
    n
}

pub struct Settings {
    pub data_dir: PathBuf,
    /// `notetaker.chromium_path`, already checked to exist.
    pub chromium: PathBuf,
    pub max_concurrent: usize,
    pub max_call_minutes: u32,
    /// `notetaker.require_chat_notice`: leave when the chat notice cannot be posted.
    pub require_chat_notice: bool,
    /// Streaming speech for the live view; `None` leaves it off.
    pub live: Option<crate::notetaker::Live>,
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
/// `max_concurrent` notetaker calls are active across all tenants. Browser calls neither count nor
/// block: the cap protects Chromium memory, which they do not use. A tenant that errors is logged and
/// skipped, so it neither blocks the others nor loses what they already claimed.
/// Slots are shared fairly: one call per workspace in turn, longest-waiting workspace first.
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
            Ok((c, notetaker, all, last)) => {
                active_total += notetaker;
                tenants.push((tenant, c, all, last));
            }
            Err(e) => tracing::error!("notetaker loop, tenant {tenant}: {e:#}"),
        }
    }
    // Fair share: the workspace that has gone longest without a call goes first, then one call per
    // workspace per round until the free slots run out or nobody has a due meeting left.
    tenants.sort_by(|a, b| (a.3, &a.0).cmp(&(b.3, &b.0)));
    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut taken = vec![0usize; tenants.len()];
    loop {
        let before = out.len();
        for (i, (tenant, c, active, _)) in tenants.iter().enumerate() {
            if active_total + out.len() >= max_concurrent {
                return Ok(out);
            }
            // `claim_scheduled` caps on this tenant's own active calls, browser ones included, and the
            // calls claimed here are `joining` already, so allow all of them plus exactly one more.
            match claim_scheduled(c, active + taken[i] + 1) {
                Ok(claimed) => {
                    taken[i] += claimed.len();
                    out.extend(
                        claimed
                            .into_iter()
                            .map(|(id, url)| (tenant.clone(), id, url)),
                    );
                }
                Err(e) => tracing::error!("notetaker loop, tenant {tenant}: {e:#}"),
            }
        }
        if out.len() == before {
            return Ok(out);
        }
    }
}

fn open_and_count(
    data_dir: &Path,
    tenant: &str,
    now: i64,
) -> Result<(rusqlite::Connection, usize, usize, i64)> {
    let c = db::open_tenant(data_dir, tenant)?;
    book_due(&c, now, LEAD_MS)?;
    // (notetaker calls, all calls) that are active.
    let (notetaker, all): (i64, i64) = c.query_row(
        "SELECT COALESCE(SUM(platform != 'browser'), 0), COUNT(*) FROM meetings
         WHERE status IN ('joining','recording')",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    // When this workspace last started a call (0 = never): who has waited longest goes first.
    // ponytail: a call claimed but not yet started has no `started_at`; the round robin covers one pass.
    let last = c.query_row(
        "SELECT COALESCE(MAX(started_at), 0) FROM meetings WHERE platform != 'browser'",
        [],
        |r| r.get(0),
    )?;
    Ok((c, notetaker as usize, all as usize, last))
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
            notice: build_notice(
                &recapz_core::settings::join_message(&c)?,
                &recapz_core::settings::notetaker_name(&c)?,
            ),
            require_notice: s.require_chat_notice,
            // Opt in with RECAPZ_NOTETAKER_DEBUG=1: a failing page step saves a screenshot.
            debug_dir: (std::env::var("RECAPZ_NOTETAKER_DEBUG").as_deref() == Ok("1"))
                .then(|| s.data_dir.join("notetaker-debug").join(&id)),
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
            s.live.as_ref(),
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
    fn slots_go_one_per_workspace_in_turn() {
        let (dir, c1) = setup();
        let control = db::open_control(dir.path()).unwrap();
        control
            .execute("INSERT INTO tenants (id, name) VALUES ('t2', 'Other')", [])
            .unwrap();
        let c2 = db::open_tenant(dir.path(), "t2").unwrap();
        for (i, id) in ["a1", "a2", "a3", "a4", "a5"].iter().enumerate() {
            meeting(&c1, id, "scheduled", 10 + i as i64);
        }
        meeting(&c2, "b1", "scheduled", 20);
        let got = due_calls(dir.path(), 2, 1_000).unwrap();
        assert_eq!(
            got.iter()
                .map(|(t, i, _)| (t.as_str(), i.as_str()))
                .collect::<Vec<_>>(),
            vec![("t1", "a1"), ("t2", "b1")]
        );
        assert_eq!(status(&c1, "a2"), "scheduled");
    }

    #[test]
    fn the_workspace_waiting_longest_goes_first() {
        let (dir, c1) = setup();
        let control = db::open_control(dir.path()).unwrap();
        control
            .execute("INSERT INTO tenants (id, name) VALUES ('t2', 'Other')", [])
            .unwrap();
        let c2 = db::open_tenant(dir.path(), "t2").unwrap();
        meeting(&c1, "old", "done", 1);
        c1.execute("UPDATE meetings SET started_at = 500 WHERE id='old'", [])
            .unwrap();
        meeting(&c1, "a", "scheduled", 10);
        meeting(&c2, "b", "scheduled", 11);
        let got = due_calls(dir.path(), 1, 1_000).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, "b", "t2 never had a call, t1 did at 500");
    }

    #[test]
    fn browser_calls_never_block_or_count_toward_notetaker_calls() {
        let (dir, c) = setup();
        for (id, status) in [("b1", "recording"), ("b2", "joining")] {
            c.execute(
                "INSERT INTO meetings (id, title, platform, status, created_at) VALUES (?1, 't', 'browser', ?2, 1)",
                [id, status],
            )
            .unwrap();
        }
        meeting(&c, "live", "recording", 2);
        meeting(&c, "a", "scheduled", 10);
        meeting(&c, "b", "scheduled", 11);
        let got = due_calls(dir.path(), 2, 1_000).unwrap();
        assert_eq!(
            got.iter().map(|(_, i, _)| i.as_str()).collect::<Vec<_>>(),
            vec!["a"],
            "two browser calls are active, yet the one notetaker call leaves room for one more"
        );
        assert_eq!(status(&c, "b"), "scheduled");
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
    fn default_settings_post_todays_text() {
        use recapz_core::settings::{DEFAULT_JOIN_MESSAGE, DEFAULT_NOTETAKER_NAME};
        assert_eq!(
            build_notice(DEFAULT_JOIN_MESSAGE, DEFAULT_NOTETAKER_NAME),
            "Recapz notetaker is recording this call and transcribing it live. \
Type 'opt out' to make it leave and delete the recording."
        );
    }

    #[test]
    fn custom_message_fills_name() {
        assert_eq!(
            build_notice("{name} records this call. Say opt out.", "Scribe"),
            "Scribe records this call. Say opt out."
        );
    }

    #[test]
    fn missing_opt_out_is_appended() {
        assert_eq!(
            build_notice("Recording.", "Scribe"),
            "Recording. Type 'opt out' to make it leave and delete the recording."
        );
        assert!(recapz_notetaker::optout::is_opt_out("opt out"));
    }
}
