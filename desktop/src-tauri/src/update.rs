//! Self-update (docs/specs/desktop-app.md, "Auto-update"). Check, download and install run here only; no
//! capability grants `updater:*`, so the remote page cannot reach the updater.
use crate::RECORDING;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::{Update, UpdaterExt};

const ENDPOINT: &str = env!("RECAPZ_UPDATE_URL");
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// An update is installed on disk; further checks this run would only offer it again.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// A prompt may appear only when nothing is recording and no update is already waiting for a restart.
fn may_prompt(recording: bool, installed: bool) -> bool {
    !recording && !installed
}

/// Checks at launch and every 6 hours, on its own thread (the dialogs block).
pub fn start(app: AppHandle) {
    thread::spawn(move || loop {
        let _ = check(&app);
        thread::sleep(EVERY);
    });
}

/// Menu item: also says when there is nothing new or the check failed.
pub fn check_now(app: AppHandle) {
    thread::spawn(move || {
        let msg = match check(&app) {
            Ok(false) => "Recapz is up to date.",
            Ok(true) => return,
            Err(()) => "Could not check for updates.",
        };
        app.dialog().message(msg).title("Recapz").blocking_show();
    });
}

fn fetch(app: &AppHandle) -> tauri_plugin_updater::Result<Option<(Update, Vec<u8>)>> {
    let url = ENDPOINT
        .parse()
        .map_err(|_| tauri_plugin_updater::Error::EmptyEndpoints)?;
    let updater = app.updater_builder().endpoints(vec![url])?.build()?;
    tauri::async_runtime::block_on(async {
        let Some(update) = updater.check().await? else {
            return Ok(None);
        };
        let bytes = update.download(|_, _| {}, || {}).await?;
        Ok(Some((update, bytes)))
    })
}

/// `Ok(true)` when an update was found and handled. Errors are only logged (no URL query, no member data).
fn check(app: &AppHandle) -> Result<bool, ()> {
    if INSTALLED.load(Ordering::Relaxed) {
        return Ok(true);
    }
    let (update, bytes) = match fetch(app) {
        Ok(Some(found)) => found,
        Ok(None) => return Ok(false),
        Err(e) => {
            eprintln!("update check failed: {e}");
            return Err(());
        }
    };
    // Never ask, and never restart, during a recording.
    while RECORDING.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_secs(10));
    }
    if !may_prompt(
        RECORDING.load(Ordering::Relaxed),
        INSTALLED.load(Ordering::Relaxed),
    ) {
        return Ok(true);
    }
    let restart = app
        .dialog()
        .message(format!("Recapz {} is ready.", update.version))
        .title("Update ready")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Restart now".into(),
            "Later".into(),
        ))
        .blocking_show();
    // A recording may have started while the dialog was open; install, but restart only when idle.
    // "Later" still installs, so the next launch is the new version.
    if let Err(e) = update.install(bytes) {
        eprintln!("update install failed: {e}");
        return Err(());
    }
    INSTALLED.store(true, Ordering::Relaxed);
    if restart && !RECORDING.load(Ordering::Relaxed) {
        app.restart();
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_only_when_idle_and_not_already_installed() {
        assert!(may_prompt(false, false));
        assert!(!may_prompt(true, false), "recording");
        assert!(!may_prompt(false, true), "already installed");
    }

    #[test]
    fn the_remote_page_is_never_granted_the_updater() {
        let cap = include_str!("../capabilities/base-url.json");
        assert!(!cap.contains("updater"), "{cap}");
    }

    #[test]
    fn the_endpoint_is_a_url() {
        assert!(ENDPOINT.parse::<tauri::Url>().is_ok(), "{ENDPOINT}");
    }
}
