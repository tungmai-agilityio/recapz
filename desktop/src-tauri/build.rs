use std::{env, fs};

/// Server this build talks to. Set `RECAPZ_BASE_URL` when building; it is baked in and cannot change later.
fn main() {
    println!("cargo:rerun-if-env-changed=RECAPZ_BASE_URL");
    let base = env::var("RECAPZ_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into());
    let base = base.trim_end_matches('/');
    println!("cargo:rustc-env=RECAPZ_BASE_URL={base}");
    // Remote IPC is allowed from this origin only (audio commands join the list in the capture tickets).
    let cap = serde_json::json!({
        "identifier": "base-url",
        "windows": ["main"],
        "remote": { "urls": [format!("{base}/*")] },
        "permissions": []
    });
    fs::create_dir_all("capabilities").unwrap();
    fs::write("capabilities/base-url.json", cap.to_string()).unwrap();
    tauri_build::build();
}
