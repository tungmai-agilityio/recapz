//! Runs the notetaker for `scheduled` meetings: claim them (capped), join through a `CallDriver`,
//! record, then hand the audio to transcription or fail the meeting with a reason.
//! A real driver and the spawned loop come with the Meet driver; this is the part that needs no browser.
use anyhow::Result;
use recapz_core::{
    db::{now_ms, tenant_dir},
    jobs,
    purge::purge_meeting,
    transcribe_job,
};
use recapz_notetaker::{
    driver::{CallDriver, DriverError, DriverEvent},
    session::{Action, Event, LeaveReason, Session},
    wav::{self, WavWriter},
};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
};

/// After a crash, less audio than this is not worth transcribing.
const MIN_CRASH_AUDIO_MS: i64 = 30_000;

/// Moves up to `max_concurrent` minus the calls already active from `scheduled` to `joining`, in one
/// transaction, and returns `(meeting id, join url)` for each. The cap counts rows, so it holds
/// across restarts.
pub fn claim_scheduled(c: &Connection, max_concurrent: usize) -> Result<Vec<(String, String)>> {
    c.execute_batch("BEGIN IMMEDIATE")?;
    let done = (|| -> Result<Vec<(String, String)>> {
        let active: usize = c.query_row(
            "SELECT COUNT(*) FROM meetings WHERE status IN ('joining','recording')",
            [],
            |r| r.get(0),
        )?;
        let room = max_concurrent.saturating_sub(active);
        let claimed = c
            .prepare(
                "SELECT id, join_url FROM meetings WHERE status = 'scheduled' AND join_url IS NOT NULL
                 ORDER BY created_at, id LIMIT ?1",
            )?
            .query_map([room as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        for (id, _) in &claimed {
            c.execute("UPDATE meetings SET status='joining' WHERE id=?1", [id])?;
        }
        Ok(claimed)
    })();
    c.execute_batch(if done.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    done
}

/// Startup recovery: a call that was `joining` failed with the server; one that was `recording`
/// has its samples on disk, so its WAV header is repaired and it is transcribed.
pub fn recover(c: &Connection, data_dir: &Path, tenant: &str) -> Result<()> {
    c.execute(
        "UPDATE meetings SET status='failed', error='server_restarted' WHERE status='joining'",
        [],
    )?;
    let stuck = c
        .prepare("SELECT id, audio_path FROM meetings WHERE status='recording'")?
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, _) in stuck {
        salvage(
            c,
            data_dir,
            tenant,
            &id,
            "The recording may be cut short because the server restarted",
            "server_restarted",
        )?;
    }
    Ok(())
}

/// One call that ended without finishing: a recording with audio on disk has its WAV header repaired
/// and is transcribed (with `note` shown on it); anything else fails with `why`. Touches only `id`,
/// so other calls running now are left alone.
pub fn salvage(
    c: &Connection,
    data_dir: &Path,
    tenant: &str,
    id: &str,
    note: &str,
    why: &str,
) -> Result<()> {
    let row: Option<(String, Option<String>)> = c
        .query_row(
            "SELECT status, audio_path FROM meetings WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((status, Some(rel))) if status == "recording" => {
            if wav::repair(&tenant_dir(data_dir, tenant).join(rel)).is_ok() {
                to_transcribing(c, id, Some(note))
            } else {
                fail(c, data_dir, tenant, id, why)
            }
        }
        Some((status, _)) if status == "joining" || status == "recording" => {
            fail(c, data_dir, tenant, id, why)
        }
        _ => Ok(()), // already finished, or gone (opt-out)
    }
}

struct Outcome {
    end: Result<LeaveReason, DriverError>,
    samples: u64,
}

/// Joins `url` and records until the session or the driver ends the call, then hands over.
/// Takes the connection by value so the future can be spawned.
#[allow(clippy::too_many_arguments)]
pub async fn run_meeting<D: CallDriver>(
    c: Connection,
    data_dir: &Path,
    tenant: &str,
    id: &str,
    url: &str,
    max_call_ms: i64,
    driver: &mut D,
    now: impl Fn() -> i64,
) -> Result<()> {
    let dir = tenant_dir(data_dir, tenant).join("audio").join(id);
    let captured = capture(&c, &dir, id, url, max_call_ms, driver, &now).await;
    driver.leave().await; // always, even when capture hit a disk error
    let out = captured?;
    if let Err(e) = &out.end {
        // The meeting id and the error variant only: never the join link or anything said in the call.
        tracing::warn!("notetaker call {id} ended with {e:?}");
    }
    let ms = (out.samples / 16) as i64;
    let enough = |min: i64, why: &str| -> Result<()> {
        if ms >= min {
            to_transcribing(&c, id, None)
        } else {
            fail(&c, data_dir, tenant, id, why)
        }
    };
    match out.end {
        Ok(LeaveReason::OptOut) => {
            purge_meeting(&c, data_dir, tenant, id, None)?; // still recording: nothing is in the store yet; the row goes too, so no status remains
            Ok(())
        }
        Ok(LeaveReason::LobbyTimeout) => {
            fail(&c, data_dir, tenant, id, "Nobody admitted the notetaker")
        }
        Ok(LeaveReason::Denied) => fail(&c, data_dir, tenant, id, "The join request was denied"),
        Ok(_) => enough(1, "No audio was captured"), // Alone, CallEnded, Removed, MaxDuration
        Err(DriverError::ChatOff) => fail(
            &c,
            data_dir,
            tenant,
            id,
            "Chat is off, so the notetaker could not announce itself",
        ),
        Err(DriverError::UiChanged(step)) => fail(
            &c,
            data_dir,
            tenant,
            id,
            &format!("meet_ui_changed: {step}"),
        ),
        Err(DriverError::SignedOut) => fail(&c, data_dir, tenant, id, "notetaker_signed_out"),
        Err(DriverError::Disconnected | DriverError::NoAudio) => enough(
            MIN_CRASH_AUDIO_MS,
            "The call dropped before enough audio was captured",
        ),
    }
}

async fn capture<D: CallDriver>(
    c: &Connection,
    dir: &Path,
    id: &str,
    url: &str,
    max_call_ms: i64,
    driver: &mut D,
    now: &impl Fn() -> i64,
) -> Result<Outcome> {
    let mut session = Session::new(now()).with_max_call_ms(max_call_ms);
    let mut samples = 0u64;
    if let Err(e) = driver.join(url).await {
        return Ok(Outcome {
            end: Err(e),
            samples,
        });
    }
    // Opened on admission: audio and speaker samples before that are not part of the call.
    let mut files: Option<(WavWriter<BufWriter<File>>, BufWriter<File>)> = None;
    let end = loop {
        match driver.next().await {
            Err(e) => break Err(e),
            Ok(DriverEvent::Call(e)) => {
                let admitted = e == Event::Admitted;
                if let Action::Leave(r) = session.on_event(e) {
                    break Ok(r);
                }
                if admitted && files.is_none() {
                    fs::create_dir_all(dir)?;
                    files = Some((
                        WavWriter::new(BufWriter::new(File::create(dir.join("recording.wav"))?))?,
                        BufWriter::new(File::create(dir.join("speakers.jsonl"))?),
                    ));
                    c.execute(
                        "UPDATE meetings SET status='recording', started_at=?2, audio_path=?3 WHERE id=?1",
                        params![id, now_ms(), format!("audio/{id}/recording.wav")],
                    )?;
                }
            }
            Ok(DriverEvent::Audio(a)) => {
                if let Some((w, _)) = &mut files {
                    w.write(&a)?;
                    samples += a.len() as u64;
                }
            }
            Ok(DriverEvent::Speaker(s)) => {
                if let Some((_, j)) = &mut files {
                    // Names are participant data: written to the call's own folder, never logged.
                    writeln!(
                        j,
                        "{}",
                        serde_json::json!({"at_ms": s.at_ms, "speaker": s.speaker})
                    )?;
                }
            }
        }
    };
    if let Some((w, mut j)) = files {
        w.finish()?.flush()?;
        j.flush()?;
    }
    Ok(Outcome { end, samples })
}

fn to_transcribing(c: &Connection, id: &str, note: Option<&str>) -> Result<()> {
    c.execute_batch("BEGIN")?;
    let done = (|| -> Result<()> {
        c.execute(
            "UPDATE meetings SET status='transcribing', ended_at=?2, error=?3 WHERE id=?1",
            params![id, now_ms(), note],
        )?;
        jobs::enqueue(c, transcribe_job::KIND, Some(id), None)?;
        Ok(())
    })();
    c.execute_batch(if done.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    done
}

/// Fails the meeting and removes any audio kept for it: a failed call has nothing to transcribe.
pub fn fail(c: &Connection, data_dir: &Path, tenant: &str, id: &str, why: &str) -> Result<()> {
    match fs::remove_dir_all(tenant_dir(data_dir, tenant).join("audio").join(id)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    c.execute(
        "UPDATE meetings SET status='failed', error=?2, audio_path=NULL, ended_at=?3 WHERE id=?1",
        params![id, why, now_ms()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use recapz_core::db;
    use recapz_notetaker::{
        driver::ScriptedDriver,
        session::{ALONE_TIMEOUT_MS, LOBBY_TIMEOUT_MS},
        speakers::Sample,
    };

    type Script = Vec<Result<DriverEvent, DriverError>>;

    fn setup() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let control = db::open_control(dir.path()).unwrap();
        db::bootstrap_admin(&control, "boss@example.com").unwrap();
        let tenant = db::tenant_ids(&control).unwrap().remove(0);
        (dir, tenant)
    }

    fn open(dir: &tempfile::TempDir, tenant: &str) -> Connection {
        db::open_tenant(dir.path(), tenant).unwrap()
    }

    fn meeting(c: &Connection, id: &str, status: &str, created: i64) {
        c.execute(
            "INSERT INTO meetings (id, title, platform, join_url, status, created_at)
             VALUES (?1, 't', 'meet', 'https://meet.example/abc', ?2, ?3)",
            params![id, status, created],
        )
        .unwrap();
    }

    fn call(e: Event) -> Result<DriverEvent, DriverError> {
        Ok(DriverEvent::Call(e))
    }

    fn audio(seconds: usize) -> Result<DriverEvent, DriverError> {
        Ok(DriverEvent::Audio(vec![7; 16_000 * seconds]))
    }

    async fn run(
        dir: &tempfile::TempDir,
        tenant: &str,
        script: Script,
        max_ms: i64,
    ) -> ScriptedDriver {
        let c = open(dir, tenant);
        meeting(&c, "m1", "joining", 0);
        let mut d = ScriptedDriver::new(script);
        run_meeting(
            c,
            dir.path(),
            tenant,
            "m1",
            "https://meet.example/abc",
            max_ms,
            &mut d,
            || 0,
        )
        .await
        .unwrap();
        d
    }

    fn row(c: &Connection) -> (String, Option<String>, Option<String>) {
        c.query_row(
            "SELECT status, error, audio_path FROM meetings WHERE id='m1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    fn transcribe_jobs(c: &Connection) -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM jobs WHERE kind='transcribe' AND meeting_id='m1'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn the_cap_counts_active_calls_and_claims_oldest_first() {
        let (dir, tenant) = setup();
        let c = open(&dir, &tenant);
        meeting(&c, "a", "recording", 1);
        meeting(&c, "b", "scheduled", 3);
        meeting(&c, "c", "scheduled", 2);
        meeting(&c, "d", "scheduled", 4);
        c.execute("INSERT INTO meetings (id, title, platform, status) VALUES ('p', 't', 'meet', 'scheduled')", [])
            .unwrap(); // no link: never claimed
        let ids = |v: Vec<(String, String)>| v.into_iter().map(|(i, _)| i).collect::<Vec<_>>();
        assert_eq!(ids(claim_scheduled(&c, 2).unwrap()), vec!["c"]);
        assert!(claim_scheduled(&c, 2).unwrap().is_empty(), "cap reached");
        assert_eq!(ids(claim_scheduled(&c, 4).unwrap()), vec!["b", "d"]);
        let joining: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM meetings WHERE status='joining'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(joining, 3);
    }

    #[tokio::test]
    async fn a_call_that_ends_is_recorded_and_queued_for_transcription() {
        let (dir, tenant) = setup();
        let s = Sample {
            at_ms: 250,
            speaker: "Ann".into(),
        };
        let d = run(
            &dir,
            &tenant,
            vec![
                audio(5), // before admission: not recorded
                call(Event::Admitted),
                audio(2),
                Ok(DriverEvent::Speaker(s)),
                audio(1),
                call(Event::CallEnded { at_ms: 9 }),
            ],
            i64::MAX,
        )
        .await;
        let c = open(&dir, &tenant);
        assert_eq!(
            row(&c),
            (
                "transcribing".into(),
                None,
                Some("audio/m1/recording.wav".into())
            )
        );
        assert_eq!(transcribe_jobs(&c), 1);
        assert_eq!(d.left, 1);
        let audio_dir = tenant_dir(dir.path(), &tenant).join("audio/m1");
        let wav = fs::read(audio_dir.join("recording.wav")).unwrap();
        assert_eq!(
            wav.len(),
            44 + 3 * 32_000,
            "three seconds of 16-bit samples"
        );
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 96_000);
        assert_eq!(
            fs::read_to_string(audio_dir.join("speakers.jsonl")).unwrap(),
            "{\"at_ms\":250,\"speaker\":\"Ann\"}\n"
        );
    }

    #[tokio::test]
    async fn opting_out_deletes_the_meeting_and_its_audio() {
        let (dir, tenant) = setup();
        let chat = Event::Chat {
            text: "opt out".into(),
            from_self: false,
        };
        run(
            &dir,
            &tenant,
            vec![call(Event::Admitted), audio(1), call(chat)],
            i64::MAX,
        )
        .await;
        let c = open(&dir, &tenant);
        let left: i64 = c
            .query_row("SELECT COUNT(*) FROM meetings", [], |r| r.get(0))
            .unwrap();
        assert_eq!((left, transcribe_jobs(&c)), (0, 0));
        assert!(!tenant_dir(dir.path(), &tenant).join("audio/m1").exists());
    }

    #[tokio::test]
    async fn each_ending_maps_to_its_status_and_message() {
        let cases: Vec<(&str, Script, i64, &str, Option<&str>)> = vec![
            (
                "chat off",
                vec![Err(DriverError::ChatOff)],
                i64::MAX,
                "failed",
                Some("Chat is off, so the notetaker could not announce itself"),
            ),
            (
                "denied",
                vec![call(Event::Denied)],
                i64::MAX,
                "failed",
                Some("The join request was denied"),
            ),
            (
                "lobby",
                vec![call(Event::Tick {
                    at_ms: LOBBY_TIMEOUT_MS,
                })],
                i64::MAX,
                "failed",
                Some("Nobody admitted the notetaker"),
            ),
            (
                "page changed",
                vec![Err(DriverError::UiChanged("join button".into()))],
                i64::MAX,
                "failed",
                Some("meet_ui_changed: join button"),
            ),
            (
                "signed out",
                vec![Err(DriverError::SignedOut)],
                i64::MAX,
                "failed",
                Some("notetaker_signed_out"),
            ),
            (
                "removed",
                vec![call(Event::Admitted), audio(3), call(Event::Removed)],
                i64::MAX,
                "transcribing",
                None,
            ),
            (
                "max duration",
                vec![
                    call(Event::Admitted),
                    audio(3),
                    call(Event::Tick { at_ms: 60_000 }),
                ],
                60_000,
                "transcribing",
                None,
            ),
            (
                "alone",
                vec![
                    call(Event::Admitted),
                    audio(3),
                    call(Event::Participants { at_ms: 0, count: 1 }),
                    call(Event::Tick {
                        at_ms: ALONE_TIMEOUT_MS,
                    }),
                ],
                i64::MAX,
                "transcribing",
                None,
            ),
            (
                "crash, 29 s",
                vec![call(Event::Admitted), audio(29)],
                i64::MAX,
                "failed",
                Some("The call dropped before enough audio was captured"),
            ),
            (
                "crash, 30 s",
                vec![call(Event::Admitted), audio(30)],
                i64::MAX,
                "transcribing",
                None,
            ),
            (
                "ended with no audio",
                vec![call(Event::Admitted), call(Event::CallEnded { at_ms: 1 })],
                i64::MAX,
                "failed",
                Some("No audio was captured"),
            ),
        ];
        for (name, script, max_ms, status, error) in cases {
            let (dir, tenant) = setup();
            run(&dir, &tenant, script, max_ms).await;
            let c = open(&dir, &tenant);
            let (s, e, path) = row(&c);
            assert_eq!((s.as_str(), e.as_deref()), (status, error), "{name}");
            let kept = status == "transcribing";
            assert_eq!(path.is_some(), kept, "{name}: audio path");
            assert_eq!(transcribe_jobs(&c), kept as i64, "{name}: transcribe job");
            assert_eq!(
                tenant_dir(dir.path(), &tenant).join("audio/m1").exists(),
                kept,
                "{name}: audio folder"
            );
        }
    }

    #[test]
    fn restart_fails_joining_and_repairs_and_transcribes_recording() {
        let (dir, tenant) = setup();
        let c = open(&dir, &tenant);
        meeting(&c, "j", "joining", 0);
        meeting(&c, "m1", "recording", 0);
        meeting(&c, "lost", "recording", 0); // audio file missing
        let path = tenant_dir(dir.path(), &tenant).join("audio/m1/recording.wav");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut w = WavWriter::new(File::create(&path).unwrap()).unwrap();
        w.write(&[1, 2, 3, 4]).unwrap(); // never finished
        drop(w);
        c.execute(
            "UPDATE meetings SET audio_path='audio/m1/recording.wav' WHERE id='m1'",
            [],
        )
        .unwrap();
        c.execute(
            "UPDATE meetings SET audio_path='audio/lost/recording.wav' WHERE id='lost'",
            [],
        )
        .unwrap();

        recover(&c, dir.path(), &tenant).unwrap();

        let get = |id: &str| -> (String, Option<String>) {
            c.query_row(
                "SELECT status, error FROM meetings WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(get("j"), ("failed".into(), Some("server_restarted".into())));
        assert_eq!(
            get("lost"),
            ("failed".into(), Some("server_restarted".into()))
        );
        assert_eq!(
            get("m1"),
            (
                "transcribing".into(),
                Some("The recording may be cut short because the server restarted".into())
            )
        );
        assert_eq!(transcribe_jobs(&c), 1);
        let b = fs::read(&path).unwrap();
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 8);
    }
}
