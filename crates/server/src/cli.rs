//! Commands run from a terminal instead of starting the server: `notetaker-login` and
//! `notetaker-logout` manage the notetaker's saved Google sign-in.
use crate::config::Config;
use anyhow::{bail, Result};
use recapz_notetaker::{browser::find_chromium, profile};
use std::path::Path;

/// Runs `command` when it is one of ours and returns `Some`; `None` means start the server.
pub fn run(command: Option<&str>, cfg: &Config) -> Option<Result<()>> {
    match command {
        Some("notetaker-login") => Some(login(cfg)),
        Some("notetaker-logout") => Some(logout(cfg)),
        _ => None,
    }
}

fn login(cfg: &Config) -> Result<()> {
    let configured = Some(cfg.notetaker.chromium_path.as_str())
        .filter(|p| !p.is_empty())
        .map(Path::new);
    let Some(chromium) = find_chromium(configured) else {
        bail!("Chromium not found: install it or set notetaker.chromium_path in the config");
    };
    if !profile::display_available(
        std::env::var("DISPLAY").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        cfg!(target_os = "linux"),
    ) {
        bail!("no display: run this over SSH with X forwarding (ssh -X) or in a VNC session");
    }
    println!("A browser window opens. Sign in with the notetaker's Google account, then close the window.");
    profile::login(&chromium, &cfg.data_dir)?;
    println!("Profile saved. Run a test call to confirm the sign-in worked.");
    Ok(())
}

fn logout(cfg: &Config) -> Result<()> {
    if profile::logout(&cfg.data_dir)? {
        println!("The notetaker's saved sign-in was deleted.");
    } else {
        println!("There was no saved sign-in.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_notetaker_commands_are_handled_and_logout_deletes_the_profile() {
        let data = std::env::temp_dir().join(format!("recapz-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let cfg = Config {
            data_dir: data.clone(),
            ..Config::default()
        };
        assert!(run(None, &cfg).is_none());
        assert!(run(Some("serve"), &cfg).is_none());

        std::fs::create_dir_all(profile::profile_dir(&data).join("Default")).unwrap();
        assert!(profile::has_login(&data));
        run(Some("notetaker-logout"), &cfg).unwrap().unwrap();
        assert!(!profile::profile_dir(&data).exists());
        run(Some("notetaker-logout"), &cfg).unwrap().unwrap(); // nothing left: still fine
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn login_names_a_missing_browser() {
        let cfg = Config {
            notetaker: crate::config::Notetaker {
                chromium_path: "/no/such/chromium".into(),
            },
            ..Config::default()
        };
        let err = run(Some("notetaker-login"), &cfg).unwrap().unwrap_err();
        assert!(err.to_string().contains("Chromium not found"), "{err}");
    }
}
