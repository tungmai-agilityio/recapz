//! Thin macOS shell: one window on the Recapz server. The page is the normal web app; Google sign-in runs in
//! the system browser (Google blocks embedded webviews) and comes back as `recapz://signed-in?code=…`.
mod audio;
mod dot;

use std::sync::Mutex;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    webview::{PageLoadEvent, WebviewWindowBuilder},
    AppHandle, Manager, RunEvent, WebviewUrl, WindowEvent,
};
use tauri_plugin_deep_link::DeepLinkExt;

/// A sign-in code waiting for the page to finish loading on our own origin.
static PENDING_CODE: Mutex<Option<(String, String)>> = Mutex::new(None);

/// The random `state` of the sign-in this app started; a deep link counts only if it carries it back.
static PENDING_STATE: Mutex<Option<String>> = Mutex::new(None);

/// Takes the pending state if `got` is it. One use: a replayed link finds nothing pending.
fn take_state(got: Option<&str>) -> Option<String> {
    let mut pending = PENDING_STATE.lock().unwrap();
    if got.is_some() && pending.as_deref() == got {
        pending.take()
    } else {
        None
    }
}

fn new_state() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    // macOS only app, so /dev/urandom is always there; no randomness crate needed.
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .expect("no randomness");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

const TRAY: &str = "recapz";

/// Shows or hides the red recording dot on the menu-bar icon.
fn set_dot(app: &AppHandle, on: bool) {
    let (Some(tray), Some(icon)) = (app.tray_by_id(TRAY), app.default_window_icon()) else {
        return;
    };
    let img = if on {
        Image::new_owned(
            dot::with_dot(icon.rgba(), icon.width(), icon.height()),
            icon.width(),
            icon.height(),
        )
    } else {
        icon.clone()
    };
    let _ = tray.set_icon(Some(img));
}

/// The page tells the shell when a recording starts and ends. The dot also goes off when the page
/// reloads or the window closes (see the hooks below), so a lost page never leaves it on.
#[tauri::command]
fn recording_state(app: AppHandle, on: bool) {
    set_dot(&app, on);
}

const BASE_URL: &str = env!("RECAPZ_BASE_URL");

/// Brings the window back, creating it on first use.
fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let google = format!("{BASE_URL}/auth/google");
    // Starts on the bundled page, which opens BASE_URL once the server answers (a dead server must not leave a blank window).
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .initialization_script(format!(
            "window.RECAPZ_BASE_URL = {};",
            serde_json::Value::String(BASE_URL.into())
        ))
        .title("Recapz")
        .inner_size(1280.0, 840.0)
        // The sign-in button goes to the system browser, signed in as a desktop sign-in.
        .on_navigation(move |u| {
            if u.path() == "/auth/google" && u.as_str().starts_with(&google) {
                let state = new_state();
                *PENDING_STATE.lock().unwrap() = Some(state.clone());
                let _ = std::process::Command::new("open")
                    .arg(format!("{BASE_URL}/auth/desktop?state={state}"))
                    .spawn();
                return false;
            }
            true
        })
        // The code is sent only from a page on our own origin, never from whatever the window shows.
        .on_page_load(|w, p| {
            // A new page has no recorder, so drop any capture the old one left running.
            if matches!(p.event(), PageLoadEvent::Started) {
                audio::stop();
                set_dot(w.app_handle(), false);
            }
            if matches!(p.event(), PageLoadEvent::Finished) && is_ours(p.url()) {
                if let Some((code, state)) = PENDING_CODE.lock().unwrap().take() {
                    exchange(&w, &code, &state);
                }
            }
        })
        .build()
        .expect("cannot open the window");
}

fn is_ours(u: &tauri::Url) -> bool {
    BASE_URL
        .parse::<tauri::Url>()
        .is_ok_and(|b| b.origin() == u.origin())
}

/// Swaps the one-time code for the session cookie (same origin, so cookie and CSRF work unchanged), then
/// reloads. A failed exchange stays on the sign-in page.
fn exchange(w: &tauri::WebviewWindow, code: &str, state: &str) {
    let js = format!(
        "fetch('/auth/desktop/exchange',{{method:'POST',headers:{{'content-type':'application/json'}},\
         body:JSON.stringify({{code:{},state:{}}})}}).then(r=>{{if(r.ok)location.assign('/')}})",
        serde_json::Value::String(code.into()),
        serde_json::Value::String(state.into())
    );
    let _ = w.eval(&js);
}

fn signed_in(app: &AppHandle, urls: &[tauri::Url]) {
    let Some((code, state)) = urls
        .iter()
        .filter(|u| u.scheme() == "recapz" && u.host_str() == Some("signed-in"))
        .find_map(|u| {
            let q = |name| {
                u.query_pairs()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.into_owned())
            };
            Some((q("code")?, q("state")?))
        })
    else {
        return;
    };
    // A link this app did not ask for (a web page can open recapz:// on its own) is ignored.
    let Some(state) = take_state(Some(&state)) else {
        return;
    };
    *PENDING_CODE.lock().unwrap() = Some((code, state));
    match app.get_webview_window("main") {
        // Reload our own origin first; the page-load hook then runs the exchange.
        Some(w) => {
            let _ = w.navigate(BASE_URL.parse().unwrap());
            show_window(app);
        }
        None => show_window(app),
    }
}

fn main() {
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            audio::audio_start,
            audio::audio_stop,
            audio::audio_permission,
            recording_state
        ])
        // A second launch (or a deep link while running) reaches the running app, which handles the URL below.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_window(app)
        }))
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            let handle = app.handle().clone();
            app.deep_link()
                .on_open_url(move |e| signed_in(&handle, &e.urls()));
            let menu = Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, "open", "Open Recapz", true, None::<&str>)?,
                    &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
                ],
            )?;
            TrayIconBuilder::with_id(TRAY)
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_window(app),
                    _ => app.exit(0),
                })
                .build(app)?;
            show_window(app.handle());
            Ok(())
        })
        // Closing minimizes: a recording window must stay alive.
        .on_window_event(|w, e| {
            if let WindowEvent::Destroyed = e {
                audio::stop();
                set_dot(w.app_handle(), false);
            }
            if let WindowEvent::CloseRequested { api, .. } = e {
                api.prevent_close();
                let _ = w.minimize();
            }
        })
        .build(tauri::generate_context!())
        .expect("cannot start Recapz");
    app.run(|app, e| {
        // Dock click with the window gone.
        if let RunEvent::Reopen { .. } = e {
            show_window(app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_counts_only_with_the_pending_state_and_only_once() {
        *PENDING_STATE.lock().unwrap() = Some("abc".into());
        assert_eq!(take_state(None), None, "no state on the link");
        assert_eq!(take_state(Some("other")), None, "wrong state");
        assert_eq!(take_state(Some("abc")), Some("abc".into()));
        assert_eq!(take_state(Some("abc")), None, "used up");
        assert_eq!(take_state(Some("")), None, "nothing pending");
    }

    #[test]
    fn states_are_32_hex_characters_and_differ() {
        let (a, b) = (new_state(), new_state());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
