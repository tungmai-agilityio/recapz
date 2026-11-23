//! Notices when the system-audio tap goes quiet mid-recording. A running tap delivers a buffer every few
//! milliseconds even for silence, so several seconds without one means it stopped or failed (the output
//! device vanished, the tap was torn down). The microphone keeps recording, so the page only has to say so.
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const STALL_MS: u64 = 3000;
const POLL: Duration = Duration::from_millis(500);

/// When the tap last delivered audio, in milliseconds since the recording started.
pub struct Beat {
    start: Instant,
    last_ms: AtomicU64,
}

impl Beat {
    pub fn new() -> Beat {
        Beat {
            start: Instant::now(),
            last_ms: AtomicU64::new(0),
        }
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Called from the audio thread for every tap buffer.
    pub fn beat(&self) {
        self.last_ms.store(self.now_ms(), Ordering::Relaxed);
    }

    fn stalled_at(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_ms.load(Ordering::Relaxed)) > STALL_MS
    }
}

/// Runs `on_stall` once if the beat stops, until dropped.
pub struct Watchdog(Arc<AtomicBool>);

impl Watchdog {
    pub fn start(beat: Arc<Beat>, on_stall: impl FnOnce() + Send + 'static) -> Watchdog {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                thread::sleep(POLL);
                if !flag.load(Ordering::Relaxed) && beat.stalled_at(beat.now_ms()) {
                    on_stall();
                    return;
                }
            }
        });
        Watchdog(done)
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalls_only_after_more_than_three_seconds_of_silence() {
        let b = Beat::new();
        b.last_ms.store(1000, Ordering::Relaxed);
        assert!(!b.stalled_at(1000));
        assert!(!b.stalled_at(4000), "exactly 3 s is still fine");
        assert!(b.stalled_at(4001));
        assert!(
            !b.stalled_at(500),
            "a clock before the last beat never stalls"
        );
    }

    #[test]
    fn a_beat_resets_the_clock() {
        let b = Beat::new();
        std::thread::sleep(Duration::from_millis(20));
        b.beat();
        assert!(b.last_ms.load(Ordering::Relaxed) >= 20);
        assert!(!b.stalled_at(b.now_ms()));
    }

    #[test]
    fn fires_once_when_the_beat_stops_and_never_after_drop() {
        let (tx, rx) = std::sync::mpsc::channel();
        // A beat that never came: the clock started ten seconds ago.
        let old = Arc::new(Beat {
            start: Instant::now() - Duration::from_secs(10),
            last_ms: AtomicU64::new(0),
        });
        let _w = Watchdog::start(old, move || tx.send(()).unwrap());
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
        assert!(
            rx.recv_timeout(Duration::from_millis(1200)).is_err(),
            "once only"
        );

        let (tx, rx) = std::sync::mpsc::channel();
        let quiet = Arc::new(Beat {
            start: Instant::now() - Duration::from_secs(10),
            last_ms: AtomicU64::new(0),
        });
        drop(Watchdog::start(quiet, move || tx.send(()).unwrap()));
        assert!(
            rx.recv_timeout(Duration::from_millis(1200)).is_err(),
            "dropped first"
        );
    }
}
