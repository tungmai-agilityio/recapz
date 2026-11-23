//! Native audio for the page: frames cross the IPC boundary as raw little-endian i16 bytes over a channel.
pub mod frame;
mod tap;

use std::sync::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};

static ACTIVE: Mutex<Option<tap::Tap>> = Mutex::new(None);

/// Starts capturing the Mac's system audio; each message on `frames` is one 1600-sample frame (3200 bytes).
#[tauri::command]
pub fn audio_start(frames: Channel<InvokeResponseBody>) -> Result<(), String> {
    let mut active = ACTIVE.lock().unwrap();
    if active.is_some() {
        return Err("already recording".into());
    }
    *active = Some(tap::Tap::start(move |f| {
        let bytes = f.iter().flat_map(|s| s.to_le_bytes()).collect();
        let _ = frames.send(InvokeResponseBody::Raw(bytes));
    })?);
    Ok(())
}

#[tauri::command]
pub fn audio_stop() {
    *ACTIVE.lock().unwrap() = None;
}

/// `"granted"`, or why not. macOS asks the member the first time, so the first call can show the prompt.
#[tauri::command]
pub fn audio_permission() -> String {
    match tap::can_tap() {
        Ok(()) => "granted".into(),
        Err(e) => e,
    }
}
