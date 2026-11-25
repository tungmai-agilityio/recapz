use std::{env, fs};

/// Server this build talks to. Set `RECAPZ_BASE_URL` when building; it is baked in and cannot change later.
fn main() {
    println!("cargo:rerun-if-env-changed=RECAPZ_BASE_URL");
    let base = env::var("RECAPZ_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into());
    let base = base.trim_end_matches('/');
    println!("cargo:rustc-env=RECAPZ_BASE_URL={base}");
    // Where the app looks for `latest.json`; a test build points it at a local server.
    println!("cargo:rerun-if-env-changed=RECAPZ_UPDATE_URL");
    let update = env::var("RECAPZ_UPDATE_URL").unwrap_or_else(|_| {
        "https://github.com/tungmai-agilityio/recapz/releases/latest/download/latest.json".into()
    });
    println!("cargo:rustc-env=RECAPZ_UPDATE_URL={update}");
    // Remote IPC is allowed from this origin only, and only for the audio commands the menu-bar recording dot and the privacy settings link.
    let cap = serde_json::json!({
        "identifier": "base-url",
        "windows": ["main"],
        "remote": { "urls": [format!("{base}/*")] },
        "permissions": ["allow-audio-start", "allow-audio-stop", "allow-audio-permission", "allow-recording-state", "allow-open-privacy-settings"]
    });
    fs::create_dir_all("capabilities").unwrap();
    fs::write("capabilities/base-url.json", cap.to_string()).unwrap();
    // The bundled meeting prompt window may only answer itself; it has no remote URL.
    let prompt = serde_json::json!({
        "identifier": "prompt",
        "windows": ["prompt"],
        "permissions": ["allow-prompt-answer"]
    });
    fs::write("capabilities/prompt.json", prompt.to_string()).unwrap();
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "audio_start",
            "audio_stop",
            "audio_permission",
            "recording_state",
            "open_privacy_settings",
            "prompt_answer",
        ]),
    ))
    .unwrap();
}
