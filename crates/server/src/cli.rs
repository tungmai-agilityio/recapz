//! Commands run from a terminal instead of starting the server: `notetaker-login` and
//! `notetaker-logout` manage the notetaker's saved Google sign-in; `workspace-create`, `workspace-list` and
//! `workspace-cap` manage workspaces.
use crate::config::Config;
use anyhow::{bail, Result};
use recapz_core::db;
use recapz_notetaker::{browser::find_chromium, profile};
use std::path::Path;

/// Runs `command` when it is one of ours and returns `Some`; `None` means start the server.
pub fn run(command: Option<&str>, cfg: &Config) -> Option<Result<()>> {
    let args: Vec<String> = std::env::args().skip(2).collect();
    run_with(command, &args, cfg)
}

fn run_with(command: Option<&str>, args: &[String], cfg: &Config) -> Option<Result<()>> {
    match command {
        Some("workspace-create") => Some(workspace_create(cfg, args)),
        Some("workspace-list") => Some(workspace_list(cfg)),
        Some("workspace-cap") => Some(workspace_cap(cfg, args)),
        Some("notetaker-login") => Some(login(cfg)),
        Some("notetaker-logout") => Some(logout(cfg)),
        _ => None,
    }
}

fn workspace_create(cfg: &Config, args: &[String]) -> Result<()> {
    let [name, email] = args else {
        bail!("usage: recapz workspace-create <name> <admin email>");
    };
    let control = db::open_control(&cfg.data_dir)?;
    let id = db::create_workspace(&control, &cfg.data_dir, name, email)?;
    println!("Workspace created ({id}). {email} joins as its admin by signing in within 14 days.");
    Ok(())
}

fn workspace_cap(cfg: &Config, args: &[String]) -> Result<()> {
    let [id, usd] = args else {
        bail!("usage: recapz workspace-cap <id> <usd> (0 = no cap)");
    };
    let usd: f64 = usd.parse()?;
    if !usd.is_finite() || usd < 0.0 {
        bail!("the cap must be zero or a positive number of USD");
    }
    let control = db::open_control(&cfg.data_dir)?;
    db::set_workspace_cap(&control, id, (usd * 1e6).round() as u64)?;
    println!("Workspace {id}: monthly cap set to ${usd:.2} (0 = no cap).");
    Ok(())
}

fn workspace_list(cfg: &Config) -> Result<()> {
    let control = db::open_control(&cfg.data_dir)?;
    println!("id\tname\tmembers\tcost this month (USD)");
    for w in db::list_workspaces(&control, &cfg.data_dir, db::now_ms())? {
        println!(
            "{}\t{}\t{}\t{:.2}",
            w.id,
            w.name,
            w.members,
            w.cost_micro_usd as f64 / 1e6
        );
    }
    Ok(())
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
    fn only_our_commands_are_handled_and_logout_deletes_the_profile() {
        let data = std::env::temp_dir().join(format!("recapz-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let cfg = Config {
            data_dir: data.clone(),
            ..Config::default()
        };
        assert!(run_with(None, &[], &cfg).is_none());
        assert!(run_with(Some("serve"), &[], &cfg).is_none());

        std::fs::create_dir_all(profile::profile_dir(&data).join("Default")).unwrap();
        assert!(profile::has_login(&data));
        run_with(Some("notetaker-logout"), &[], &cfg)
            .unwrap()
            .unwrap();
        assert!(!profile::profile_dir(&data).exists());
        run_with(Some("notetaker-logout"), &[], &cfg)
            .unwrap()
            .unwrap(); // nothing left: still fine
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
        let err = run_with(Some("notetaker-login"), &[], &cfg)
            .unwrap()
            .unwrap_err();
        assert!(err.to_string().contains("Chromium not found"), "{err}");
    }

    #[test]
    fn workspace_commands_create_refuse_a_duplicate_and_list() {
        let data = std::env::temp_dir().join(format!("recapz-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let cfg = Config {
            data_dir: data.clone(),
            ..Config::default()
        };
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        run_with(Some("workspace-create"), &a(&["Acme", "boss@x.com"]), &cfg)
            .unwrap()
            .unwrap();
        let err = run_with(Some("workspace-create"), &a(&["acme", "o@x.com"]), &cfg)
            .unwrap()
            .unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        let err = run_with(Some("workspace-create"), &a(&["Only name"]), &cfg)
            .unwrap()
            .unwrap_err();
        assert!(err.to_string().contains("usage"), "{err}");
        run_with(Some("workspace-list"), &[], &cfg)
            .unwrap()
            .unwrap();
        let control = db::open_control(&data).unwrap();
        assert_eq!(
            db::list_workspaces(&control, &data, db::now_ms())
                .unwrap()
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(&data);
    }
}
