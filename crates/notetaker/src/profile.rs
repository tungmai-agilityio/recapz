//! The notetaker's saved browser profile: a person signs the notetaker's Google account in once in a
//! visible window, and every call copies that profile (see `browser`). The profile folder holds
//! live session cookies, so it is private to the owner, never logged, excluded from backups by the
//! deploy setup, and hard deleted by `logout`. Unix only.
use anyhow::{bail, Context, Result};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

const SIGN_IN_URL: &str = "https://accounts.google.com/";

pub fn profile_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("notetaker-profile")
}

/// Whether a person has signed in: the browser has written something into the profile.
pub fn has_login(data_dir: &Path) -> bool {
    profile_dir(data_dir).join("Default").is_dir()
}

/// What the last check of the saved profile found. `Unknown` = a profile exists but no call has
/// reached Meet with it yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignIn {
    SignedIn,
    SignedOut,
    Unknown,
}

impl SignIn {
    pub fn as_str(self) -> &'static str {
        match self {
            SignIn::SignedIn => "signed_in",
            SignIn::SignedOut => "signed_out",
            SignIn::Unknown => "unknown",
        }
    }
}

fn status_file(data_dir: &Path) -> PathBuf {
    data_dir.join("notetaker-signin") // beside the profile, never inside it: no cookies here
}

/// Notes what a call just found: Meet showed the sign-in page, or let the account in.
pub fn record_sign_in(data_dir: &Path, found: SignIn) {
    let _ = fs::write(status_file(data_dir), found.as_str()); // a status note must never fail a call
}

/// Signed out when there is no profile or the last call found the sign-in page; signed in when the
/// last call got past it.
pub fn sign_in(data_dir: &Path) -> SignIn {
    if !has_login(data_dir) {
        return SignIn::SignedOut;
    }
    match fs::read_to_string(status_file(data_dir)).as_deref() {
        Ok("signed_out") => SignIn::SignedOut,
        Ok("signed_in") => SignIn::SignedIn,
        _ => SignIn::Unknown,
    }
}

/// Whether a window can be shown: always on macOS and Windows, on Linux only with a display
/// (the server runs a virtual one; a person needs SSH X forwarding or VNC to reach it).
pub fn display_available(display: Option<&str>, wayland: Option<&str>, linux: bool) -> bool {
    !linux || display.is_some_and(|d| !d.is_empty()) || wayland.is_some_and(|d| !d.is_empty())
}

/// Opens a visible Chromium on the persistent profile and waits until the person closes it. No
/// debug port is opened for this window: a person types the password, nothing automates it.
pub fn login(chromium: &Path, data_dir: &Path) -> Result<()> {
    let dir = profile_dir(data_dir);
    if fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("the profile folder is a symlink; remove it first");
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?; // also fixes a folder made earlier
    let _ = fs::remove_file(status_file(data_dir)); // a fresh sign-in has not been checked yet
    let status = Command::new(chromium)
        .arg(format!("--user-data-dir={}", dir.display()))
        .args(["--no-first-run", "--no-default-browser-check", SIGN_IN_URL])
        .status()
        .with_context(|| format!("could not start {}", chromium.display()))?;
    if !status.success() {
        bail!("the browser closed with an error ({status})");
    }
    if !has_login(data_dir) {
        bail!("the browser left no profile behind; sign in and close the window normally");
    }
    Ok(())
}

/// Removes the saved login (hard delete). Returns whether there was one.
pub fn logout(data_dir: &Path) -> Result<bool> {
    let _ = fs::remove_file(status_file(data_dir));
    match fs::remove_dir_all(profile_dir(data_dir)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("recapz-profile-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// A stand-in browser: records its arguments, and writes a cookie file like a real sign-in.
    fn fake_browser(dir: &Path, writes_profile: bool, exit: i32) -> PathBuf {
        let script = dir.join("fake-browser.sh");
        let body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{args}'\n{write}exit {exit}\n",
            args = dir.join("args.txt").display(),
            write = if writes_profile {
                "mkdir -p \"${1#--user-data-dir=}/Default\" && echo cookie > \"${1#--user-data-dir=}/Default/Cookies\"\n"
            } else {
                ""
            },
        );
        fs::write(&script, body).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    fn sign_in_follows_the_profile_and_the_last_call_and_a_fresh_login_resets_it() {
        let data = tmp("signin");
        assert_eq!(sign_in(&data), SignIn::SignedOut); // no profile
        login(&fake_browser(&data, true, 0), &data).unwrap();
        assert_eq!(sign_in(&data), SignIn::Unknown); // profile, never checked
        record_sign_in(&data, SignIn::SignedIn);
        assert_eq!(sign_in(&data), SignIn::SignedIn);
        record_sign_in(&data, SignIn::SignedOut);
        assert_eq!(sign_in(&data), SignIn::SignedOut);
        login(&fake_browser(&data, true, 0), &data).unwrap();
        assert_eq!(sign_in(&data), SignIn::Unknown);
        record_sign_in(&data, SignIn::SignedIn);
        logout(&data).unwrap();
        assert_eq!(sign_in(&data), SignIn::SignedOut);
        assert!(!status_file(&data).exists());
    }

    #[test]
    fn login_makes_a_private_profile_runs_a_visible_browser_without_a_debug_port_and_logout_deletes_it(
    ) {
        let data = tmp("login");
        assert!(!has_login(&data));
        // A folder made earlier with loose permissions is tightened.
        fs::create_dir_all(profile_dir(&data)).unwrap();
        fs::set_permissions(profile_dir(&data), fs::Permissions::from_mode(0o755)).unwrap();

        login(&fake_browser(&data, true, 0), &data).unwrap();

        let mode = fs::metadata(profile_dir(&data))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        let args = fs::read_to_string(data.join("args.txt")).unwrap();
        let lines: Vec<&str> = args.lines().collect();
        assert_eq!(
            lines[0],
            format!("--user-data-dir={}", profile_dir(&data).display())
        );
        assert_eq!(lines.last(), Some(&SIGN_IN_URL));
        assert!(
            !lines
                .iter()
                .any(|l| l.contains("headless") || l.contains("remote-debugging")),
            "a person signs in: visible, and nothing can drive it"
        );
        assert!(has_login(&data));

        assert!(logout(&data).unwrap());
        assert!(!profile_dir(&data).exists() && !has_login(&data));
        assert!(!logout(&data).unwrap(), "logging out twice is harmless");
        fs::remove_dir_all(&data).unwrap();
    }

    #[test]
    fn a_browser_that_fails_or_leaves_nothing_is_not_a_login() {
        let data = tmp("fail");
        assert!(login(&fake_browser(&data, true, 3), &data).is_err());
        let data2 = tmp("empty");
        assert!(login(&fake_browser(&data2, false, 0), &data2).is_err());
        assert!(!has_login(&data2));
        assert!(login(Path::new("/no/such/browser"), &data2).is_err());
        let _ = (fs::remove_dir_all(&data), fs::remove_dir_all(&data2));
    }

    #[test]
    fn a_symlinked_profile_folder_is_refused() {
        let data = tmp("link");
        let elsewhere = tmp("elsewhere");
        std::os::unix::fs::symlink(&elsewhere, profile_dir(&data)).unwrap();
        assert!(login(&fake_browser(&data, true, 0), &data).is_err());
        assert!(!has_login(&elsewhere));
        let _ = (fs::remove_dir_all(&data), fs::remove_dir_all(&elsewhere));
    }

    #[test]
    fn a_window_needs_a_display_on_linux_only() {
        assert!(display_available(None, None, false));
        assert!(display_available(Some(":0"), None, true));
        assert!(display_available(None, Some("wayland-0"), true));
        assert!(!display_available(None, None, true));
        assert!(!display_available(Some(""), Some(""), true));
    }
}
