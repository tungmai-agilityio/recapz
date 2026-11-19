//! A call fed by audio from a member's browser: `BrowserDriver` is a `CallDriver` that is admitted at
//! once and posts no chat notice (the recorder told the meeting themselves). Frames and stop requests
//! arrive through a per-meeting registry, so the recording socket and the call never share a task.
//! The real call code runs on top of it (recording, the live transcript, then the transcript and
//! report jobs), like the local replay in `dev_call`.
use crate::notetaker::{run_meeting, salvage, Live};
use anyhow::Result;
use recapz_core::db::{self, now_ms};
use recapz_notetaker::{
    driver::{CallDriver, DriverError, DriverEvent},
    session::Event,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::{interval, sleep_until, Instant, Interval, MissedTickBehavior},
};

/// How long the call stays open without a frame, so a page can reconnect and resend what it buffered.
pub const QUIET: Duration = Duration::from_secs(60);
/// How often the call's clock ticks, so the hard cap on call length applies with no other events.
const TICK: Duration = Duration::from_secs(1);

/// How the recorder ends a call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stop {
    /// End the call: what was recorded is transcribed.
    End,
    /// End it and delete the meeting, its audio and any live text, as the chat opt-out does.
    Discard,
}

enum Msg {
    /// A (re)connected socket: its frames replace the previous socket's.
    Attach(UnboundedReceiver<Vec<i16>>),
    Stop(Stop),
}

type Calls = Mutex<HashMap<String, UnboundedSender<Msg>>>;

fn calls() -> &'static Calls {
    static CALLS: OnceLock<Calls> = OnceLock::new();
    CALLS.get_or_init(Default::default)
}

/// A sender for the frames of the browser call running under `id`, replacing the sender of any
/// earlier socket without ending the call. `None` when no browser call runs under `id`.
pub fn attach(id: &str) -> Option<UnboundedSender<Vec<i16>>> {
    let (tx, rx) = unbounded_channel();
    let calls = calls().lock().unwrap();
    calls.get(id)?.send(Msg::Attach(rx)).ok()?;
    Some(tx)
}

/// Stops the browser call running under `id`; false when none runs.
pub fn stop(id: &str, how: Stop) -> bool {
    calls()
        .lock()
        .unwrap()
        .get(id)
        .is_some_and(|tx| tx.send(Msg::Stop(how)).is_ok())
}

pub struct BrowserDriver {
    id: String,
    /// Kept to unregister only this call, never a later one under the same id.
    me: UnboundedSender<Msg>,
    msgs: UnboundedReceiver<Msg>,
    frames: Option<UnboundedReceiver<Vec<i16>>>,
    /// A stop or reconnect waiting until the frames queued before it are delivered.
    pending: Option<Msg>,
    admitted: bool,
    quiet: Duration,
    last_frame: Instant,
    tick: Interval,
}

impl BrowserDriver {
    /// Registers a browser call under `id`; `attach` and `stop` reach it until it is dropped.
    pub fn register(id: &str) -> Self {
        Self::with_quiet(id, QUIET)
    }

    fn with_quiet(id: &str, quiet: Duration) -> Self {
        let (tx, msgs) = unbounded_channel();
        calls().lock().unwrap().insert(id.to_string(), tx.clone());
        let mut tick = interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            id: id.to_string(),
            me: tx,
            msgs,
            frames: None,
            pending: None,
            admitted: false,
            quiet,
            last_frame: Instant::now(),
            tick,
        }
    }
}

impl Drop for BrowserDriver {
    fn drop(&mut self) {
        let mut calls = calls().lock().unwrap();
        if calls
            .get(&self.id)
            .is_some_and(|tx| tx.same_channel(&self.me))
        {
            calls.remove(&self.id);
        }
    }
}

async fn frame(rx: &mut Option<UnboundedReceiver<Vec<i16>>>) -> Option<Vec<i16>> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

impl CallDriver for BrowserDriver {
    async fn join(&mut self, _url: &str) -> Result<(), DriverError> {
        Ok(())
    }

    async fn next(&mut self) -> Result<DriverEvent, DriverError> {
        if !self.admitted {
            self.admitted = true;
            self.last_frame = Instant::now();
            return Ok(DriverEvent::Call(Event::Admitted));
        }
        loop {
            if let Some(m) = self.pending.take() {
                // Frames and stops travel apart: deliver what the socket sent before this first.
                if let Some(f) = self.frames.as_mut().and_then(|rx| rx.try_recv().ok()) {
                    self.pending = Some(m);
                    self.last_frame = Instant::now();
                    return Ok(DriverEvent::Audio(f));
                }
                match m {
                    Msg::Attach(rx) => self.frames = Some(rx),
                    Msg::Stop(Stop::End) => {
                        return Ok(DriverEvent::Call(Event::CallEnded { at_ms: now_ms() }))
                    }
                    Msg::Stop(Stop::Discard) => {
                        return Ok(DriverEvent::Call(Event::Chat {
                            text: "opt out".into(),
                            from_self: false,
                        }))
                    }
                }
                continue;
            }
            tokio::select! {
                biased;
                m = self.msgs.recv() => self.pending = Some(m.unwrap_or(Msg::Stop(Stop::End))),
                f = frame(&mut self.frames) => match f {
                    Some(f) => {
                        self.last_frame = Instant::now();
                        return Ok(DriverEvent::Audio(f));
                    }
                    None => self.frames = None, // the socket closed; wait for a reconnect
                },
                _ = sleep_until(self.last_frame + self.quiet) => return Err(DriverError::NoAudio),
                _ = self.tick.tick() => {
                    return Ok(DriverEvent::Call(Event::Tick { at_ms: now_ms() }))
                }
            }
        }
    }

    async fn leave(&mut self) {}
}

/// Runs the browser call for the existing `joining` meeting `id` on a thread and runtime of its own,
/// like a notetaker call. It is registered before this returns, so a socket can attach at once. The
/// meeting is settled even when the call fails.
pub fn spawn(
    data_dir: PathBuf,
    tenant: String,
    id: String,
    live: Option<Live>,
    max_call_ms: i64,
) -> std::io::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("browser-call".into())
        .spawn(move || {
            let result: Result<()> = (|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(async {
                    let mut driver = BrowserDriver::register(&id);
                    let _ = tx.send(());
                    let c = db::open_tenant(&data_dir, &tenant)?;
                    run_meeting(
                        c,
                        &data_dir,
                        &tenant,
                        &id,
                        "browser",
                        max_call_ms,
                        &mut driver,
                        live.as_ref(),
                        now_ms,
                    )
                    .await
                })
            })();
            if let Err(e) = result {
                // The meeting id only: never anything said in the call.
                tracing::error!("browser call {id} failed: {e:#}");
                let settled = db::open_tenant(&data_dir, &tenant).and_then(|c| {
                    salvage(
                        &c,
                        &data_dir,
                        &tenant,
                        &id,
                        "The recording may be cut short because the call ended with an error",
                        "browser_call_error",
                    )
                });
                if let Err(e) = settled {
                    tracing::error!("browser call {id} could not be settled: {e:#}");
                }
            }
        })?;
    // Err only when the thread died before registering; its error path already settled the meeting.
    let _ = rx.recv();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The next event that is not a clock tick.
    async fn next(d: &mut BrowserDriver) -> Result<DriverEvent, DriverError> {
        loop {
            match d.next().await {
                Ok(DriverEvent::Call(Event::Tick { .. })) => continue,
                other => return other,
            }
        }
    }

    #[tokio::test]
    async fn admitted_at_once_with_no_chat_notice_then_frames_in_order() {
        let mut d = BrowserDriver::register("b-frames");
        d.join("browser").await.unwrap();
        assert_eq!(next(&mut d).await, Ok(DriverEvent::Call(Event::Admitted)));
        let tx = attach("b-frames").unwrap();
        tx.send(vec![1; 1600]).unwrap();
        tx.send(vec![2; 1600]).unwrap();
        assert_eq!(next(&mut d).await, Ok(DriverEvent::Audio(vec![1; 1600])));
        assert_eq!(next(&mut d).await, Ok(DriverEvent::Audio(vec![2; 1600])));
    }

    #[tokio::test]
    async fn a_reconnect_swaps_the_sender_without_ending_the_call() {
        let mut d = BrowserDriver::register("b-swap");
        next(&mut d).await.unwrap();
        let old = attach("b-swap").unwrap();
        old.send(vec![1]).unwrap();
        assert_eq!(next(&mut d).await, Ok(DriverEvent::Audio(vec![1])));
        drop(old); // the socket dropped
        let new = attach("b-swap").unwrap();
        new.send(vec![2]).unwrap();
        assert_eq!(next(&mut d).await, Ok(DriverEvent::Audio(vec![2])));
        // A socket that is still open but replaced is no longer heard.
        let newer = attach("b-swap").unwrap();
        newer.send(vec![3]).unwrap();
        assert_eq!(next(&mut d).await, Ok(DriverEvent::Audio(vec![3])));
        assert!(new.send(vec![9]).is_err(), "the replaced socket is cut off");
    }

    #[tokio::test]
    async fn frames_sent_before_a_stop_or_a_reconnect_are_all_delivered_first() {
        let mut d = BrowserDriver::register("b-order");
        next(&mut d).await.unwrap();
        let old = attach("b-order").unwrap();
        old.send(vec![1]).unwrap();
        old.send(vec![2]).unwrap();
        let new = attach("b-order").unwrap();
        new.send(vec![3]).unwrap();
        assert!(stop("b-order", Stop::End));
        for want in [1, 2, 3] {
            assert_eq!(next(&mut d).await, Ok(DriverEvent::Audio(vec![want])));
        }
        assert!(matches!(
            next(&mut d).await,
            Ok(DriverEvent::Call(Event::CallEnded { .. }))
        ));
    }

    #[tokio::test]
    async fn stop_ends_the_call_and_discard_is_the_opt_out() {
        let mut d = BrowserDriver::register("b-stop");
        next(&mut d).await.unwrap();
        assert!(stop("b-stop", Stop::End));
        assert!(matches!(
            next(&mut d).await,
            Ok(DriverEvent::Call(Event::CallEnded { .. }))
        ));
        assert!(stop("b-stop", Stop::Discard));
        let Ok(DriverEvent::Call(Event::Chat { text, from_self })) = next(&mut d).await else {
            panic!("discard must arrive as a chat opt-out");
        };
        assert!(!from_self);
        assert!(recapz_notetaker::optout::is_opt_out(&text));
    }

    #[tokio::test]
    async fn quiet_for_the_limit_ends_with_no_audio_and_frames_reset_it() {
        let quiet = Duration::from_millis(200);
        let mut d = BrowserDriver::with_quiet("b-quiet", quiet);
        next(&mut d).await.unwrap();
        let tx = attach("b-quiet").unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
        tx.send(vec![1]).unwrap();
        assert_eq!(next(&mut d).await, Ok(DriverEvent::Audio(vec![1])));
        let from = Instant::now();
        assert_eq!(next(&mut d).await, Err(DriverError::NoAudio));
        assert!(
            from.elapsed() >= Duration::from_millis(150),
            "the frame restarted the wait"
        );
    }

    /// Runs a browser call for meeting `id` under `run_meeting`; `feed` drives it from outside.
    async fn run_call(
        id: &str,
        quiet: Duration,
        feed: impl std::future::Future<Output = ()>,
    ) -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let control = db::open_control(dir.path()).unwrap();
        db::bootstrap_admin(&control, "boss@example.com").unwrap();
        let tenant = db::tenant_ids(&control).unwrap().remove(0);
        let c = db::open_tenant(dir.path(), &tenant).unwrap();
        c.execute(
            "INSERT INTO meetings (id, title, platform, status, created_at) VALUES (?1, 't', 'browser', 'joining', 0)",
            [id],
        )
        .unwrap();
        let mut d = BrowserDriver::with_quiet(id, quiet);
        let call = run_meeting(
            db::open_tenant(dir.path(), &tenant).unwrap(),
            dir.path(),
            &tenant,
            id,
            "browser",
            i64::MAX,
            &mut d,
            None,
            now_ms,
        );
        let (done, ()) = tokio::join!(call, feed);
        done.unwrap();
        (dir, c)
    }

    fn row(c: &rusqlite::Connection, id: &str) -> Option<String> {
        c.query_row("SELECT status FROM meetings WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .ok()
    }

    /// 31 s of audio, over the minimum a call that drops is transcribed with.
    fn send_31s(tx: &UnboundedSender<Vec<i16>>) {
        for _ in 0..310 {
            tx.send(vec![7; 1600]).unwrap();
        }
    }

    #[tokio::test]
    async fn a_stopped_call_is_transcribed_and_a_discarded_one_deleted() {
        let (_d, c) = run_call("b-run-stop", QUIET, async {
            tokio::task::yield_now().await;
            send_31s(&attach("b-run-stop").unwrap());
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(stop("b-run-stop", Stop::End));
        })
        .await;
        assert_eq!(row(&c, "b-run-stop").as_deref(), Some("transcribing"));

        let (_d, c) = run_call("b-run-discard", QUIET, async {
            tokio::task::yield_now().await;
            send_31s(&attach("b-run-discard").unwrap());
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(stop("b-run-discard", Stop::Discard));
        })
        .await;
        assert_eq!(row(&c, "b-run-discard"), None, "the meeting row is gone");
    }

    #[tokio::test]
    async fn a_call_gone_quiet_transcribes_what_it_recorded() {
        let (_d, c) = run_call("b-run-quiet", Duration::from_millis(200), async {
            tokio::task::yield_now().await;
            let tx = attach("b-run-quiet").unwrap();
            send_31s(&tx);
            // The socket stays open but sends nothing more.
            tokio::time::sleep(Duration::from_millis(600)).await;
            drop(tx);
        })
        .await;
        assert_eq!(row(&c, "b-run-quiet").as_deref(), Some("transcribing"));
    }

    #[test]
    fn nothing_reaches_a_call_that_is_not_running_or_was_dropped() {
        assert!(attach("b-none").is_none());
        assert!(!stop("b-none", Stop::End));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = rt.enter();
        let first = BrowserDriver::register("b-drop");
        let second = BrowserDriver::register("b-drop"); // a later call under the same id
        drop(first);
        assert!(attach("b-drop").is_some(), "the later call stays reachable");
        drop(second);
        assert!(attach("b-drop").is_none());
    }
}
