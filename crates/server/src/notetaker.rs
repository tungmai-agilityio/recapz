//! Runs the notetaker for `scheduled` meetings: claim them (capped), join through a `CallDriver`,
//! record, then hand the audio to transcription or fail the meeting with a reason.
//! A real driver and the spawned loop come with the Meet driver; this is the part that needs no browser.
use crate::live::{hub, Msg};
use anyhow::Result;
use recapz_core::{
    db::{now_ms, tenant_dir},
    jobs,
    purge::purge_meeting,
    transcribe_job,
};
use recapz_notetaker::{
    driver::{CallDriver, DriverError, DriverEvent},
    profile::{self, SignIn},
    session::{Action, Event, LeaveReason, Session},
    wav::{self, WavWriter},
};
use recapz_speech::stream::{DeepgramStream, LiveEvent};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    sync::mpsc::Receiver,
};

/// After a crash, less audio than this is not worth transcribing.
const MIN_CRASH_AUDIO_MS: i64 = 30_000;

/// Moves up to `max_concurrent` minus the calls already active from `scheduled` to `joining`, in one
/// transaction, and returns `(meeting id, join url)` for each. The cap counts rows, so it holds
/// across restarts.
pub fn claim_scheduled(c: &Connection, max_concurrent: usize) -> Result<Vec<(String, String)>> {
    c.execute_batch("BEGIN IMMEDIATE")?;
    let done = (|| -> Result<Vec<(String, String)>> {
        let active: i64 = c.query_row(
            "SELECT COUNT(*) FROM meetings WHERE status IN ('joining','recording')",
            [],
            |r| r.get(0),
        )?;
        let room = max_concurrent.saturating_sub(active as usize);
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

/// Where the call's audio is streamed for the live view. `None` turns the live transcript off.
pub struct Live {
    pub base_url: String,
    pub api_key: String,
}

/// Moves what the stream has produced so far into the live view. `at_ms` is how far into the call we are.
fn pump(id: &str, rx: &Receiver<LiveEvent>, at_ms: i64) {
    while let Ok(e) = rx.try_recv() {
        hub().push(
            id,
            match e {
                LiveEvent::Partial(t) => Msg::Partial(t),
                LiveEvent::Final(text) => Msg::Final { at_ms, text },
                LiveEvent::Unavailable => Msg::Unavailable,
            },
        );
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
    live: Option<&Live>,
    now: impl Fn() -> i64,
) -> Result<()> {
    let dir = tenant_dir(data_dir, tenant).join("audio").join(id);
    let captured = capture(&c, &dir, id, url, max_call_ms, driver, live, &now).await;
    hub().clear(id); // the live transcript never outlives the call, whatever ended it
    driver.leave().await; // always, even when capture hit a disk error
    let out = captured?;
    if let Err(e) = &out.end {
        // The meeting id and the error variant only: never the join link or anything said in the call.
        tracing::warn!("notetaker call {id} ended with {e:?}");
    }
    match &out.end {
        Err(DriverError::SignedOut) => profile::record_sign_in(data_dir, SignIn::SignedOut),
        Ok(_) => profile::record_sign_in(data_dir, SignIn::SignedIn), // reached the call: the account is in
        Err(_) => {}
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

#[allow(clippy::too_many_arguments)]
async fn capture<D: CallDriver>(
    c: &Connection,
    dir: &Path,
    id: &str,
    url: &str,
    max_call_ms: i64,
    driver: &mut D,
    live: Option<&Live>,
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
    let mut stream: Option<(DeepgramStream, Receiver<LiveEvent>)> = None;
    let end = loop {
        let event = driver.next().await;
        // The live view's controls: stop leaves (the audio so far goes to transcription), pause mutes.
        let ctl = hub().control(id);
        if ctl.stop {
            if let Action::Leave(r) = session.on_event(Event::Stop) {
                break Ok(r);
            }
        }
        match event {
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
                    if let Some(l) = live {
                        let lang: String =
                            c.query_row("SELECT language FROM meetings WHERE id=?1", [id], |r| {
                                r.get(0)
                            })?;
                        hub().start(id);
                        let (s, rx) = DeepgramStream::start(&l.base_url, &l.api_key, &lang);
                        stream = Some((s, rx));
                    }
                }
            }
            Ok(DriverEvent::Audio(a)) => {
                if let Some((w, _)) = &mut files {
                    if ctl.paused {
                        // Nothing from the call is written or streamed; silence of the same length (also sent to the stream, so the link stays open) keeps
                        // the recording's clock aligned with the live one, and shows as a gap on the report.
                        // ponytail: the silence is transcribed (and billed) like any audio.
                        let quiet = vec![0; a.len()];
                        w.write(&quiet)?;
                        if let Some((s, _)) = &stream {
                            s.send_frame(&quiet); // keeps the live link open through a long pause
                        }
                    } else {
                        w.write(&a)?;
                        if let Some((s, _)) = &stream {
                            s.send_frame(&a); // a dropped frame only costs live text, never the recording
                        }
                    }
                    samples += a.len() as u64;
                    if let Some((_, rx)) = &stream {
                        pump(id, rx, (samples / 16) as i64);
                    }
                }
            }
            Ok(DriverEvent::Speaker(s)) => {
                if let (Some((_, j)), false) = (&mut files, ctl.paused) {
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
        run_with(dir, tenant, "m1", script, max_ms, None).await
    }

    /// The live store is shared by the whole process, so tests that look at it use their own meeting id.
    async fn run_with(
        dir: &tempfile::TempDir,
        tenant: &str,
        id: &str,
        script: Script,
        max_ms: i64,
        live: Option<&Live>,
    ) -> ScriptedDriver {
        let c = open(dir, tenant);
        meeting(&c, id, "joining", 0);
        let mut d = ScriptedDriver::new(script);
        run_meeting(
            c,
            dir.path(),
            tenant,
            id,
            "https://meet.example/abc",
            max_ms,
            &mut d,
            live,
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
            "SELECT COUNT(*) FROM jobs WHERE kind='transcribe'",
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

    #[test]
    fn stream_events_reach_the_live_view_stamped_with_the_call_clock() {
        hub().start("pump1");
        let (tx, rx) = std::sync::mpsc::channel();
        for e in [
            LiveEvent::Partial("hel".into()),
            LiveEvent::Final("hello".into()),
            LiveEvent::Partial("wor".into()),
            LiveEvent::Unavailable,
        ] {
            tx.send(e).unwrap();
        }
        pump("pump1", &rx, 5_000);
        let s = hub().subscribe("pump1").unwrap();
        assert_eq!(s.lines, vec![(5_000, "hello".to_string())]);
        assert_eq!((s.partial.as_str(), s.unavailable), ("wor", true));
        hub().clear("pump1");
    }

    /// A speech link that cannot connect: nothing is streamed, yet the call is recorded in full.
    fn dead_link() -> Live {
        Live {
            base_url: "ws://127.0.0.1:1/v1/listen".into(),
            api_key: "k".into(),
        }
    }

    #[tokio::test]
    async fn a_failed_live_stream_does_not_stop_the_recording_and_leaves_nothing_behind() {
        let (dir, tenant) = setup();
        run_with(
            &dir,
            &tenant,
            "live-dead",
            vec![
                call(Event::Admitted),
                audio(2),
                call(Event::CallEnded { at_ms: 9 }),
            ],
            i64::MAX,
            Some(&dead_link()),
        )
        .await;
        let c = open(&dir, &tenant);
        let status: String = c
            .query_row(
                "SELECT status FROM meetings WHERE id='live-dead'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "transcribing");
        assert_eq!(transcribe_jobs(&c), 1);
        assert!(
            hub().subscribe("live-dead").is_none(),
            "cleared when the call ended"
        );
    }

    /// A scripted call where the live view's controls are used at chosen steps (1 is the first event).
    struct Steered {
        inner: ScriptedDriver,
        step: usize,
        acts: Vec<(usize, Box<dyn Fn()>)>,
    }

    impl CallDriver for Steered {
        async fn join(&mut self, url: &str) -> Result<(), DriverError> {
            self.inner.join(url).await
        }
        async fn next(&mut self) -> Result<DriverEvent, DriverError> {
            self.step += 1;
            for (at, act) in &self.acts {
                if *at == self.step {
                    act();
                }
            }
            self.inner.next().await
        }
        async fn leave(&mut self) {
            self.inner.leave().await
        }
    }

    /// Runs `script` as meeting `id` with the live transcript on; returns the driver and the recording's samples.
    async fn steered(
        id: &str,
        script: Script,
        acts: Vec<(usize, Box<dyn Fn()>)>,
    ) -> (Steered, Vec<i16>, String) {
        let (dir, tenant) = setup();
        let c = open(&dir, &tenant);
        meeting(&c, id, "joining", 0);
        let mut d = Steered {
            inner: ScriptedDriver::new(script),
            step: 0,
            acts,
        };
        run_meeting(
            c,
            dir.path(),
            &tenant,
            id,
            "https://meet.example/abc",
            i64::MAX,
            &mut d,
            Some(&dead_link()),
            || 0,
        )
        .await
        .unwrap();
        let bytes = fs::read(
            tenant_dir(dir.path(), &tenant)
                .join("audio")
                .join(id)
                .join("recording.wav"),
        )
        .unwrap();
        let samples = bytes[44..]
            .chunks(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let status = open(&dir, &tenant)
            .query_row("SELECT status FROM meetings WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        (d, samples, status)
    }

    #[tokio::test]
    async fn a_paused_call_writes_silence_instead_of_the_call_and_resumes() {
        let script = vec![
            call(Event::Admitted),
            audio(1),
            audio(1),
            audio(1),
            call(Event::CallEnded { at_ms: 9 }),
        ];
        let acts: Vec<(usize, Box<dyn Fn()>)> = vec![
            (
                3,
                Box::new(|| {
                    hub().set_paused("ctl-pause", true);
                }),
            ),
            (
                4,
                Box::new(|| {
                    hub().set_paused("ctl-pause", false);
                }),
            ),
        ];
        let (_, samples, status) = steered("ctl-pause", script, acts).await;
        assert_eq!(status, "transcribing");
        let sec = |i: usize| {
            samples[i * 16_000..(i + 1) * 16_000]
                .iter()
                .all(|s| *s == if i == 1 { 0 } else { 7 })
        };
        assert_eq!(samples.len(), 48_000);
        assert!(
            sec(0) && sec(1) && sec(2),
            "second one is silence, the others are the call"
        );
    }

    #[tokio::test]
    async fn a_stop_request_leaves_keeps_the_audio_so_far_and_goes_to_transcription() {
        let script = vec![
            call(Event::Admitted),
            audio(1),
            audio(1),
            audio(1),
            call(Event::CallEnded { at_ms: 9 }),
        ];
        let acts: Vec<(usize, Box<dyn Fn()>)> = vec![(
            3,
            Box::new(|| {
                hub().request_stop("ctl-stop");
            }),
        )];
        let (d, samples, status) = steered("ctl-stop", script, acts).await;
        assert_eq!(
            (status.as_str(), samples.len(), d.inner.left),
            ("transcribing", 16_000, 1)
        );
        assert!(samples.iter().all(|s| *s == 7));
        assert!(
            hub().subscribe("ctl-stop").is_none(),
            "cleared when the call ended"
        );
    }

    #[tokio::test]
    async fn opting_out_clears_the_live_transcript_as_well() {
        let (dir, tenant) = setup();
        let chat = Event::Chat {
            text: "opt out".into(),
            from_self: false,
        };
        run_with(
            &dir,
            &tenant,
            "live-optout",
            vec![call(Event::Admitted), audio(1), call(chat)],
            i64::MAX,
            Some(&dead_link()),
        )
        .await;
        assert!(hub().subscribe("live-optout").is_none());
        let c = open(&dir, &tenant);
        let left: i64 = c
            .query_row("SELECT COUNT(*) FROM meetings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
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

    #[tokio::test]
    async fn a_call_notes_whether_the_account_was_signed_in() {
        let cases: Vec<(&str, Script, SignIn)> = vec![
            (
                "sign-in page",
                vec![Err(DriverError::SignedOut)],
                SignIn::SignedOut,
            ),
            (
                "denied, so past the sign-in page",
                vec![call(Event::Denied)],
                SignIn::SignedIn,
            ),
            (
                "page changed says nothing",
                vec![Err(DriverError::UiChanged("x".into()))],
                SignIn::Unknown,
            ),
        ];
        for (name, script, want) in cases {
            let (dir, tenant) = setup();
            std::fs::create_dir_all(profile::profile_dir(dir.path()).join("Default")).unwrap();
            assert_eq!(
                profile::sign_in(dir.path()),
                SignIn::Unknown,
                "{name}: before"
            );
            run(&dir, &tenant, script, i64::MAX).await;
            assert_eq!(profile::sign_in(dir.path()), want, "{name}");
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
