//! "Record this meeting?" (docs/specs/meeting-detection.md, ticket 2). A detected call opens a small window from
//! the bundled `ui/prompt.html`; its buttons answer through `prompt_answer`, which only that window may call.
//! Record hands the call to the page, which opens the Record dialog, so nothing records without the member.
use crate::detect::Event;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tauri::{webview::WebviewWindowBuilder, AppHandle, Manager, WebviewUrl};

const WINDOW: &str = "prompt";
const WIDTH: f64 = 360.0;
const HEIGHT: f64 = 128.0;

/// Kept on this Mac only, in the app's config folder.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Prefs {
    #[serde(default = "on")]
    pub detect: bool,
    #[serde(default)]
    pub never: Vec<String>,
}

fn on() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            detect: true,
            never: Vec::new(),
        }
    }
}

impl Prefs {
    /// A missing or unreadable file means the defaults: detection on, no app silenced.
    pub fn load(path: &Path) -> Prefs {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, serde_json::to_vec(self)?)
        };
        if let Err(e) = write() {
            eprintln!("meeting detection: cannot save settings: {e}");
        }
    }

    /// Asks only with detection on, nothing recording, and the app not silenced.
    pub fn should_ask(&self, recording: bool, app: &str) -> bool {
        self.detect && !recording && !self.never.iter().any(|a| a == app)
    }
}

static PREFS: Mutex<Option<Prefs>> = Mutex::new(None);
/// The app the open prompt asks about.
static ASKING: Mutex<Option<String>> = Mutex::new(None);
/// A Record answer waiting for the page to load on our own origin.
pub static PENDING: Mutex<Option<String>> = Mutex::new(None);
/// Read by the detection thread; off means no polling at all.
pub static ENABLED: AtomicBool = AtomicBool::new(true);

fn prefs_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("detect.json"))
}

fn with_prefs<T>(app: &AppHandle, f: impl FnOnce(&mut Prefs) -> T, save: bool) -> T {
    let mut guard = PREFS.lock().unwrap();
    let path = prefs_path(app);
    let prefs = guard.get_or_insert_with(|| path.as_deref().map(Prefs::load).unwrap_or_default());
    let out = f(prefs);
    if save {
        if let Some(p) = &path {
            prefs.save(p);
        }
    }
    out
}

/// Loads the settings at launch; returns whether detection is on (the menu item's tick).
pub fn init(app: &AppHandle) -> bool {
    let on = with_prefs(app, |p| p.detect, false);
    ENABLED.store(on, Ordering::Relaxed);
    on
}

pub fn set_enabled(app: &AppHandle, on: bool) {
    with_prefs(app, |p| p.detect = on, true);
    ENABLED.store(on, Ordering::Relaxed);
    if !on {
        close(app);
    }
}

/// "Ask again for all apps" in the menu bar.
pub fn forget_never(app: &AppHandle) {
    with_prefs(app, |p| p.never.clear(), true);
}

/// Called on the detection thread for every call start and end.
pub fn on_event(app: &AppHandle, e: Event) {
    match e {
        Event::Started(name) => {
            let recording = crate::RECORDING.load(Ordering::Relaxed);
            if with_prefs(app, |p| p.should_ask(recording, name), false) {
                let app2 = app.clone();
                let _ = app.run_on_main_thread(move || open(&app2, name));
            }
        }
        Event::Ended(name) => {
            if ASKING.lock().unwrap().as_deref() == Some(name) {
                close(app);
            }
            dispatch(app, "recapz-meeting-ended", name);
        }
    }
}

fn open(app: &AppHandle, name: &str) {
    close(app);
    *ASKING.lock().unwrap() = Some(name.into());
    // Top right of the main screen, under the menu bar.
    let x = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.size().to_logical::<f64>(m.scale_factor()).width - WIDTH - 16.0)
        .unwrap_or(0.0);
    let built = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("prompt.html".into()))
        .initialization_script(format!(
            "window.RECAPZ_PROMPT_APP = {};",
            serde_json::Value::String(name.into())
        ))
        .title("Record this meeting?")
        .inner_size(WIDTH, HEIGHT)
        .position(x, 40.0)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        .build();
    if let Err(e) = built {
        eprintln!("meeting detection: cannot open the prompt: {e}");
    }
}

fn close(app: &AppHandle) {
    *ASKING.lock().unwrap() = None;
    if let Some(w) = app.get_webview_window(WINDOW) {
        let _ = w.destroy();
    }
}

/// Tells the page about the call. A page not on our own origin (still loading, server down) gets it once it
/// loads, through `PENDING`, for a Record answer only.
fn dispatch(app: &AppHandle, event: &str, name: &str) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    if w.url().is_ok_and(|u| crate::is_ours(&u)) {
        let _ = w.eval(script(event, name));
    } else if event == "recapz-meeting-detected" {
        *PENDING.lock().unwrap() = Some(name.into());
    }
}

pub fn script(event: &str, name: &str) -> String {
    format!(
        "window.dispatchEvent(new CustomEvent({}, {{ detail: {{ app: {} }} }}))",
        serde_json::Value::String(event.into()),
        serde_json::Value::String(name.into())
    )
}

/// The prompt's buttons: `record`, `never`, anything else (Ignore, the 30 s timeout) just closes it.
#[tauri::command]
pub fn prompt_answer(app: AppHandle, answer: String) {
    let Some(name) = ASKING.lock().unwrap().clone() else {
        return;
    };
    close(&app);
    match answer.as_str() {
        "record" => {
            // The page may need loading first; it then finds the call in PENDING.
            if app.get_webview_window("main").is_none() {
                *PENDING.lock().unwrap() = Some(name);
                crate::show_window(&app);
            } else {
                crate::show_window(&app);
                dispatch(&app, "recapz-meeting-detected", &name);
            }
        }
        "never" => with_prefs(&app, |p| p.never.push(name), true),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_only_when_on_idle_and_not_silenced() {
        let mut p = Prefs::default();
        assert!(p.should_ask(false, "Zoom"));
        assert!(!p.should_ask(true, "Zoom"), "never while recording");
        p.never.push("Zoom".into());
        assert!(!p.should_ask(false, "Zoom"));
        assert!(p.should_ask(false, "Slack"));
        p.detect = false;
        assert!(!p.should_ask(false, "Slack"));
    }

    #[test]
    fn settings_default_on_and_round_trip() {
        let dir = std::env::temp_dir().join(format!("recapz-prefs-{}", std::process::id()));
        let path = dir.join("detect.json");
        assert_eq!(Prefs::load(&path), Prefs::default(), "no file");
        assert!(Prefs::default().detect, "on by default");
        let p = Prefs {
            detect: false,
            never: vec!["Zoom".into()],
        };
        p.save(&path);
        assert_eq!(Prefs::load(&path), p);
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(Prefs::load(&path), Prefs::default(), "missing keys");
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(Prefs::load(&path), Prefs::default(), "broken file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_page_event_quotes_the_app_name() {
        assert_eq!(
            script("recapz-meeting-detected", "Zoom\"x"),
            r#"window.dispatchEvent(new CustomEvent("recapz-meeting-detected", { detail: { app: "Zoom\"x" } }))"#
        );
    }

    #[test]
    fn only_the_prompt_window_may_answer() {
        let remote = include_str!("../capabilities/base-url.json");
        assert!(!remote.contains("prompt-answer"));
        let local = include_str!("../capabilities/prompt.json");
        assert!(local.contains(r#""windows":["prompt"]"#));
        assert!(!local.contains("remote"));
    }
}
