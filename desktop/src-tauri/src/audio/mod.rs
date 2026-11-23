pub mod frame;
mod mic;
mod mix;
mod tap;
mod watch;

use serde::Deserialize;
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};

/// `Meeting` records the Mac's sound and the microphone, `InRoom` the microphone only.
#[derive(Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Meeting,
    InRoom,
}

/// Whatever is running; dropping it stops everything.
struct Session {
    _tap: Option<tap::Tap>,
    _mic: mic::Mic,
    _watch: Option<watch::Watchdog>,
}

static ACTIVE: Mutex<Option<Session>> = Mutex::new(None);

// Commands are async so Tauri runs them off the main thread: capture can block on a permission prompt.
/// Starts capturing; each message on `frames` is one 1600-sample frame (3200 bytes). Without `mode` it records
/// a meeting.
#[tauri::command]
pub async fn audio_start(
    window: tauri::WebviewWindow,
    frames: Channel<InvokeResponseBody>,
    mode: Option<Mode>,
) -> Result<(), String> {
    // A page that reloaded lost its channel, so a new start replaces a stale session instead of failing.
    let mut active = ACTIVE.lock().unwrap();
    *active = None;
    let send = move |f: &[i16]| {
        let bytes = f.iter().flat_map(|s| s.to_le_bytes()).collect();
        let _ = frames.send(InvokeResponseBody::Raw(bytes));
    };
    *active = Some(match mode.unwrap_or_default() {
        Mode::InRoom => Session {
            _tap: None,
            _mic: mic::Mic::start(Arc::new(send))?,
            _watch: None,
        },
        Mode::Meeting => {
            // ponytail: one mutex shared by both audio threads, held for one frame's arithmetic.
            let mixer = Arc::new(Mutex::new(mix::Mixer::default()));
            let send = Arc::new(send);
            let side = |n: usize| {
                let (mixer, send) = (mixer.clone(), send.clone());
                move |f: &[i16]| mixer.lock().unwrap().push(n, f, &*send)
            };
            let beat = Arc::new(watch::Beat::new());
            let tap_side = side(0);
            let tap_beat = beat.clone();
            let tap = tap::Tap::start(move |f: &[i16]| {
                tap_beat.beat();
                tap_side(f)
            })?;
            // The page cannot listen for app events (the capability allows commands only), so it gets a DOM event.
            let watch = watch::Watchdog::start(beat, move || {
                let _ = window
                    .eval("window.dispatchEvent(new CustomEvent('recapz-system-audio-stopped'))");
            });
            Session {
                _tap: Some(tap),
                _mic: mic::Mic::start(Arc::new(side(1)))?,
                _watch: Some(watch),
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn audio_stop() {
    stop();
}

/// Also called when the page reloads or the window goes away, so a lost page never leaves a capture running.
pub fn stop() {
    *ACTIVE.lock().unwrap() = None;
}

/// `"granted"`, or a comma-separated list of what is missing: `microphone`, `audio_capture` (the Mac's sound).
/// In-room needs only the microphone. macOS asks the member the first time, so the first call can show prompts.
#[tauri::command]
pub async fn audio_permission(mode: Option<Mode>) -> String {
    let mut missing = Vec::new();
    if mic::can_record().is_err() {
        missing.push("microphone");
    }
    if matches!(mode.unwrap_or_default(), Mode::Meeting) && tap::can_tap().is_err() {
        missing.push("audio_capture");
    }
    if missing.is_empty() {
        "granted".into()
    } else {
        missing.join(",")
    }
}
