//! Local-only fake call that replays a recording as if it were a live Meet call: `FileDriver` is a
//! `CallDriver` that is admitted at once, sends the audio as 100 ms frames in real time, then
//! ends the call. The real call code runs on top of it (recording, the live transcript over the
//! speech provider's streaming API, then the transcript and report jobs); only the browser is
//! missing. Started by `routes::dev_live`, which refuses unless `dev_login` is on.
use crate::notetaker::{run_meeting, salvage, Live};
use anyhow::Result;
use recapz_core::db::{self, now_ms};
use recapz_notetaker::{
    driver::{CallDriver, DriverError, DriverEvent},
    session::Event,
};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::{interval, Interval, MissedTickBehavior},
};

/// 100 ms of 16 kHz mono audio, the frame size the notetaker sends.
const FRAME: usize = 1600;

/// How a running fake call is stopped early.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stop {
    /// Leave now, as if the call ended: what was recorded is transcribed.
    End,
    /// Type "opt out" in chat: the meeting and its audio are deleted.
    OptOut,
}

type Stops = Mutex<HashMap<String, UnboundedSender<Stop>>>;

fn stops() -> &'static Stops {
    static STOPS: OnceLock<Stops> = OnceLock::new();
    STOPS.get_or_init(Default::default)
}

/// Stops a running fake call; false when none runs under `id`.
pub fn stop(id: &str, how: Stop) -> bool {
    stops()
        .lock()
        .unwrap()
        .get(id)
        .is_some_and(|tx| tx.send(how).is_ok())
}

struct FileDriver {
    frames: VecDeque<Vec<i16>>,
    admitted: bool,
    sent_ms: i64,
    stops: UnboundedReceiver<Stop>,
    tick: Interval,
}

impl CallDriver for FileDriver {
    async fn join(&mut self, _url: &str) -> Result<(), DriverError> {
        Ok(())
    }

    async fn next(&mut self) -> Result<DriverEvent, DriverError> {
        if !self.admitted {
            self.admitted = true;
            return Ok(DriverEvent::Call(Event::Admitted));
        }
        tokio::select! {
            Some(how) = self.stops.recv() => Ok(DriverEvent::Call(match how {
                Stop::End => Event::CallEnded { at_ms: self.sent_ms },
                Stop::OptOut => Event::Chat { text: "opt out".into(), from_self: false },
            })),
            _ = self.tick.tick() => Ok(DriverEvent::Call(Event::CallEnded { at_ms: self.sent_ms }))
                .map(|ended| match self.frames.pop_front() {
                    Some(f) => {
                        self.sent_ms += 100;
                        DriverEvent::Audio(f)
                    }
                    None => ended,
                }),
        }
    }

    async fn leave(&mut self) {}
}

/// 16 kHz mono samples as 16-bit, in 100 ms frames.
fn frames(samples: &[f32]) -> VecDeque<Vec<i16>> {
    samples
        .chunks(FRAME)
        .map(|c| {
            c.iter()
                .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect()
        })
        .collect()
}

/// Plays `samples` into the existing `joining` meeting `id`, on a thread and runtime of its own like
/// a real call. The meeting is settled even when the call fails.
pub fn spawn(
    data_dir: PathBuf,
    tenant: String,
    id: String,
    samples: Vec<f32>,
    live: Live,
    max_call_ms: i64,
) -> std::io::Result<()> {
    let (tx, rx) = unbounded_channel();
    stops().lock().unwrap().insert(id.clone(), tx);
    let queued = id.clone();
    let started = std::thread::Builder::new()
        .name("dev-call".into())
        .spawn(move || {
            let result: Result<()> = (|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(async {
                    let mut tick = interval(Duration::from_millis(100));
                    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
                    let mut driver = FileDriver {
                        frames: frames(&samples),
                        admitted: false,
                        sent_ms: 0,
                        stops: rx,
                        tick,
                    };
                    let c = db::open_tenant(&data_dir, &tenant)?;
                    run_meeting(
                        c,
                        &data_dir,
                        &tenant,
                        &id,
                        "dev-simulator",
                        max_call_ms,
                        &mut driver,
                        Some(&live),
                        now_ms,
                    )
                    .await
                })
            })();
            if let Err(e) = result {
                // The meeting id only: never anything said in the call.
                tracing::error!("simulated call {id} failed: {e:#}");
                let settled = db::open_tenant(&data_dir, &tenant).and_then(|c| {
                    salvage(
                        &c,
                        &data_dir,
                        &tenant,
                        &id,
                        "The simulated call stopped early",
                        "The simulated call failed",
                    )
                });
                if let Err(e) = settled {
                    tracing::error!("simulated call {id} could not be settled: {e:#}");
                }
            }
            stops().lock().unwrap().remove(&id);
        });
    if started.is_err() {
        stops().lock().unwrap().remove(&queued);
    }
    started.map(|_| ())
}
