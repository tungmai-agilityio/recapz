//! Native audio for the page: frames cross the IPC boundary as raw little-endian i16 bytes over a channel.
pub mod frame;
mod tap;

use std::sync::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};

static ACTIVE: Mutex<Option<tap::Tap>> = Mutex::new(None);

// Commands are async so Tauri runs them off the main thread: the tap can block on the permission prompt.
/// Starts capturing the Mac's system audio; each message on `frames` is one 1600-sample frame (3200 bytes).
#[tauri::command]
pub async fn audio_start(frames: Channel<InvokeResponseBody>) -> Result<(), String> {
    // A page that reloaded lost its channel, so a new start replaces a stale session instead of failing.
    let mut active = ACTIVE.lock().unwrap();
    *active = None;
    *active = Some(tap::Tap::start(move |f| {
        let bytes = f.iter().flat_map(|s| s.to_le_bytes()).collect();
        let _ = frames.send(InvokeResponseBody::Raw(bytes));
    })?);
    Ok(())
}

#[tauri::command]
pub async fn audio_stop() {
    stop();
}

/// Also called when the page reloads or the window goes away, so a lost page never leaves the tap running.
pub fn stop() {
    *ACTIVE.lock().unwrap() = None;
}

/// `"granted"`, or why not. macOS asks the member the first time, so the first call can show the prompt.
#[tauri::command]
pub async fn audio_permission() -> String {
    match tap::can_tap() {
        Ok(()) => "granted".into(),
        Err(e) => e,
    }
}
