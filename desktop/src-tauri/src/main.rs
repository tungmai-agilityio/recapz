//! Thin macOS shell: one window on the Recapz server. The page is the normal web app; Google sign-in runs in
//! the system browser (Google blocks embedded webviews) and comes back as `recapz://signed-in?code=…`.
mod audio;

use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    webview::{PageLoadEvent, WebviewWindowBuilder},
    AppHandle, Manager, RunEvent, WebviewUrl, WindowEvent,
};
use tauri_plugin_deep_link::DeepLinkExt;

/// A sign-in code waiting for the page to finish loading on our own origin.
static PENDING_CODE: Mutex<Option<String>> = Mutex::new(None);

const BASE_URL: &str = env!("RECAPZ_BASE_URL");

/// Brings the window back, creating it on first use.
fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let url = BASE_URL.parse().expect("RECAPZ_BASE_URL is not a URL");
    let google = format!("{BASE_URL}/auth/google");
    WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
        .title("Recapz")
        .inner_size(1280.0, 840.0)
        // The sign-in button goes to the system browser, signed in as a desktop sign-in.
        .on_navigation(move |u| {
            if u.path() == "/auth/google" && u.as_str().starts_with(&google) {
                let _ = std::process::Command::new("open")
                    .arg(format!("{BASE_URL}/auth/desktop"))
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
            }
            if matches!(p.event(), PageLoadEvent::Finished) && is_ours(p.url()) {
                if let Some(code) = PENDING_CODE.lock().unwrap().take() {
                    exchange(&w, &code);
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
fn exchange(w: &tauri::WebviewWindow, code: &str) {
    let js = format!(
        "fetch('/auth/desktop/exchange',{{method:'POST',headers:{{'content-type':'application/json'}},\
         body:JSON.stringify({{code:{}}})}}).then(r=>{{if(r.ok)location.assign('/')}})",
        serde_json::Value::String(code.into())
    );
    let _ = w.eval(&js);
}

fn signed_in(app: &AppHandle, urls: &[tauri::Url]) {
    let Some(code) = urls
        .iter()
        .filter(|u| u.scheme() == "recapz" && u.host_str() == Some("signed-in"))
        .find_map(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "code")
                .map(|(_, v)| v.into_owned())
        })
    else {
        return;
    };
    *PENDING_CODE.lock().unwrap() = Some(code);
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
            audio::audio_permission
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
            TrayIconBuilder::new()
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
