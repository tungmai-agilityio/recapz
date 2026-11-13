//! Starts one Chromium per call and guarantees it is gone afterwards: its whole process group is
//! killed and the per-call profile copy (a credential) is deleted when the value is dropped, on
//! every path including errors and panics. Unix only.
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use std::{
    fs,
    os::unix::{fs::DirBuilderExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const START_TIMEOUT: Duration = Duration::from_secs(20);
/// Not copied from the saved profile: locks and the stale debug port would confuse the new
/// browser, and caches only slow the copy down.
const SKIP: &[&str] = &[
    "DevToolsActivePort",
    "Cache",
    "Code Cache",
    "GPUCache",
    "DawnCache",
    "GrShaderCache",
];

/// A child started in its own process group, so renderer and GPU helpers die with it.
pub struct ProcessGroup {
    child: Child,
    work_dir: PathBuf,
}

impl ProcessGroup {
    /// Spawns `cmd` and takes ownership of `work_dir`, which is deleted on drop (also when the
    /// spawn fails).
    pub fn spawn(cmd: &mut Command, work_dir: PathBuf) -> Result<Self> {
        match cmd
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => Ok(Self { child, work_dir }),
            Err(e) => {
                let _ = fs::remove_dir_all(&work_dir);
                Err(e.into())
            }
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // ponytail: shells out to kill(1) instead of adding a libc dependency for one killpg call.
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.child.id())])
            .stderr(Stdio::null())
            .status();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.work_dir);
    }
}

/// Copies a saved browser profile, leaving out the entries in `SKIP` and anything that is not a
/// plain file or folder. The new folders are private to the owner.
pub fn copy_profile(src: &Path, dst: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let name = e.file_name();
        let n = name.to_string_lossy();
        if n.starts_with("Singleton") || SKIP.contains(&n.as_ref()) {
            continue;
        }
        let ty = e.file_type()?;
        if ty.is_dir() {
            copy_profile(&e.path(), &dst.join(&name))?;
        } else if ty.is_file() {
            fs::copy(e.path(), dst.join(&name))?;
        }
    }
    Ok(())
}

/// The configured path when set (it must exist: a wrong path is an error, not a fallback),
/// otherwise the first usual location that exists.
pub fn find_chromium(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured {
        return p.is_file().then(|| p.to_path_buf());
    }
    [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "/snap/bin/chromium",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

pub struct Launch {
    /// `notetaker.chromium_path`; `None` searches the usual locations.
    pub chromium: Option<PathBuf>,
    /// The logged-in profile to copy for this call; `None` starts with an empty one.
    pub profile: Option<PathBuf>,
    /// A folder that does not exist yet and is unique to this call; deleted with the browser.
    pub work_dir: PathBuf,
    /// Headless is for tests; the server runs a visible window on a virtual display.
    pub headless: bool,
}

pub struct Browser {
    pub inner: chromiumoxide::Browser,
    handler: tokio::task::JoinHandle<()>,
    group: ProcessGroup,
}

impl Browser {
    pub async fn launch(o: Launch) -> Result<Self> {
        let exe = find_chromium(o.chromium.as_deref()).context("Chromium not found")?;
        if o.work_dir.exists() {
            bail!("work dir already exists");
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&o.work_dir)?;
        let user_data = o.work_dir.join("profile");
        let prepared = match &o.profile {
            Some(src) => copy_profile(src, &user_data),
            None => Ok(fs::DirBuilder::new().mode(0o700).create(&user_data)?),
        };
        if let Err(e) = prepared {
            let _ = fs::remove_dir_all(&o.work_dir);
            return Err(e);
        }
        let mut cmd = Command::new(exe);
        cmd.arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", user_data.display()))
            // No real camera or microphone is ever opened, and the notetaker sends no audio.
            .args([
                "--use-fake-device-for-media-stream",
                "--use-fake-ui-for-media-stream",
            ])
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "about:blank",
            ]);
        if o.headless {
            cmd.arg("--headless=new");
        }
        // From here, dropping `group` on any error kills the browser and removes the work dir.
        let group = ProcessGroup::spawn(&mut cmd, o.work_dir)?;
        let port = debug_port(&user_data.join("DevToolsActivePort")).await?;
        let (inner, mut events) =
            chromiumoxide::Browser::connect(format!("http://127.0.0.1:{port}"))
                .await
                .context("could not connect to the browser")?;
        let handler = tokio::spawn(async move { while events.next().await.is_some() {} });
        Ok(Self {
            inner,
            handler,
            group,
        })
    }

    pub fn pid(&self) -> u32 {
        self.group.pid()
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.handler.abort(); // the process group guard then kills the browser
    }
}

/// Chromium writes its debug port to this file in the profile folder once it is listening.
async fn debug_port(file: &Path) -> Result<u16> {
    let start = Instant::now();
    while start.elapsed() < START_TIMEOUT {
        if let Some(port) = fs::read_to_string(file)
            .ok()
            .and_then(|s| s.lines().next().and_then(|l| l.trim().parse().ok()))
        {
            return Ok(port);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("the browser did not start listening in time")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alive(pid: &str) -> bool {
        Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    fn wait_for(f: impl Fn() -> bool) -> bool {
        for _ in 0..100 {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("recapz-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn dropping_the_group_kills_its_helpers_and_removes_the_work_dir() {
        let work = tmp("group");
        fs::create_dir_all(&work).unwrap();
        let pidfile = work.join("helper.pid");
        // A parent that starts a helper of its own, like a browser's renderer.
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(format!("sleep 300 & echo $! > {}; wait", pidfile.display()));
        let g = ProcessGroup::spawn(&mut cmd, work.clone()).unwrap();
        assert!(wait_for(
            || fs::read_to_string(&pidfile).is_ok_and(|s| !s.trim().is_empty())
        ));
        let helper = fs::read_to_string(&pidfile).unwrap().trim().to_string();
        let parent = g.pid().to_string();
        assert!(alive(&helper) && alive(&parent));
        drop(g);
        assert!(wait_for(|| !alive(&helper)), "helper still running");
        assert!(!alive(&parent));
        assert!(!work.exists());
    }

    #[test]
    fn a_failed_spawn_still_removes_the_work_dir() {
        let work = tmp("nospawn");
        fs::create_dir_all(&work).unwrap();
        assert!(ProcessGroup::spawn(&mut Command::new("/no/such/binary"), work.clone()).is_err());
        assert!(!work.exists());
    }

    #[test]
    fn the_profile_copy_skips_locks_the_stale_port_and_caches_but_keeps_logins() {
        let (src, dst) = (tmp("psrc"), tmp("pdst"));
        fs::create_dir_all(src.join("Default/Cache")).unwrap();
        fs::create_dir_all(src.join("Default/Network")).unwrap();
        for f in [
            "Local State",
            "Default/Cookies",
            "Default/Network/state",
            "Default/Cache/x",
            "SingletonLock",
            "DevToolsActivePort",
        ] {
            fs::write(src.join(f), f).unwrap();
        }
        copy_profile(&src, &dst).unwrap();
        let mut got: Vec<String> = Vec::new();
        for d in [&dst, &dst.join("Default"), &dst.join("Default/Network")] {
            for e in fs::read_dir(d).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_file() {
                    got.push(e.path().strip_prefix(&dst).unwrap().display().to_string());
                }
            }
        }
        got.sort();
        assert_eq!(
            got,
            ["Default/Cookies", "Default/Network/state", "Local State"]
        );
        assert_eq!(
            fs::read_to_string(dst.join("Default/Cookies")).unwrap(),
            "Default/Cookies"
        );
        let _ = (fs::remove_dir_all(&src), fs::remove_dir_all(&dst));
    }

    #[test]
    fn a_configured_chromium_path_must_exist_and_is_never_replaced_by_a_search() {
        let file = tmp("exe");
        fs::write(&file, "").unwrap();
        assert_eq!(find_chromium(Some(&file)), Some(file.clone()));
        assert_eq!(find_chromium(Some(Path::new("/no/such/chromium"))), None);
        fs::remove_file(file).unwrap();
    }
}

/// Needs a real Chromium; run with `make test-browser`. Prints and returns when none is found.
#[cfg(all(test, feature = "browser-tests"))]
mod browser_tests {
    use super::*;

    #[tokio::test]
    async fn launches_connects_uses_a_copy_of_the_profile_and_cleans_up() {
        if find_chromium(None).is_none() {
            println!("SKIPPED: no Chromium found");
            return;
        }
        let root = std::env::temp_dir().join(format!("recapz-browser-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let saved = root.join("saved");
        fs::create_dir_all(&saved).unwrap();
        fs::write(saved.join("marker"), "login").unwrap();
        let work = root.join("call-1");

        let b = Browser::launch(Launch {
            chromium: None,
            profile: Some(saved.clone()),
            work_dir: work.clone(),
            headless: true,
        })
        .await
        .unwrap();
        let version = b.inner.version().await.unwrap().product;
        assert!(version.contains("Chrome"), "{version}");
        b.inner.new_page("about:blank").await.unwrap();
        assert_eq!(
            fs::read_to_string(work.join("profile/marker")).unwrap(),
            "login"
        );
        let pid = b.pid().to_string();
        assert!(Command::new("kill")
            .args(["-0", &pid])
            .status()
            .unwrap()
            .success());

        drop(b);
        assert!(!work.exists(), "profile copy must be deleted");
        let mut gone = false;
        for _ in 0..100 {
            if !Command::new("kill")
                .args(["-0", &pid])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
            {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(gone, "browser still running");
        assert!(
            saved.join("marker").exists(),
            "the saved profile is untouched"
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
