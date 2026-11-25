//! Meeting detection (docs/specs/meeting-detection.md): which meeting app holds the microphone right now.
//! Core Audio lists every process that does audio and says whether it is reading input (macOS 14+), so this
//! needs no permission and never opens the microphone itself.
use objc2_core_audio::*;
use objc2_core_foundation::{CFRetained, CFString};
use std::{
    collections::BTreeMap, ffi::c_void, ptr::NonNull, thread, time::Duration, time::Instant,
};

/// A call has started once the app held the microphone this long without a break.
const START_MS: u64 = 5_000;
/// A started call has ended once the app released the microphone this long (a short release is not the end).
const END_MS: u64 = 15_000;
const POLL: Duration = Duration::from_secs(2);

/// Bundle ids of apps that hold the microphone for calls. A helper process (`com.google.Chrome.helper`) counts
/// as its app. Any other app (Voice Memos, dictation) never counts.
const APPS: &[(&str, &str)] = &[
    ("us.zoom.xos", "Zoom"),
    ("com.microsoft.teams2", "Microsoft Teams"),
    ("com.microsoft.teams", "Microsoft Teams"),
    ("com.tinyspeck.slackmacgap", "Slack"),
    ("com.cisco.webexmeetingsapp", "Webex"),
    ("com.apple.FaceTime", "FaceTime"),
    ("com.hnc.Discord", "Discord"),
    ("com.google.Chrome", "Google Chrome"),
    ("com.microsoft.edgemac", "Microsoft Edge"),
    ("company.thebrowser.Browser", "Arc"),
    ("com.apple.Safari", "Safari"),
    // ponytail: Safari pages use the microphone through the shared WebKit GPU process, so another WebKit app's
    // microphone also reads as Safari; map by the process's parent if that misfires.
    ("com.apple.WebKit.GPU", "Safari"),
];

/// The meeting app a bundle id belongs to, if any.
pub fn app_of(bundle: &str) -> Option<&'static str> {
    APPS.iter().find_map(|&(id, name)| {
        let b = bundle.as_bytes();
        let hit = b.len() >= id.len()
            && b[..id.len()].eq_ignore_ascii_case(id.as_bytes())
            && (b.len() == id.len() || b[id.len()] == b'.');
        hit.then_some(name)
    })
}

#[derive(Debug, PartialEq)]
pub enum Event {
    Started(&'static str),
    Ended(&'static str),
}

struct Seen {
    since: u64,
    last: u64,
    started: bool,
}

/// Turns "who holds the microphone now" samples into call start and end events per app.
#[derive(Default)]
pub struct Detector(BTreeMap<&'static str, Seen>);

impl Detector {
    /// `now` is in milliseconds; `holding` the meeting apps holding the microphone at that moment.
    pub fn tick(&mut self, now: u64, holding: &[&'static str]) -> Vec<Event> {
        let mut out = Vec::new();
        for &app in holding {
            let s = self.0.entry(app).or_insert(Seen {
                since: now,
                last: now,
                started: false,
            });
            s.last = now;
            if !s.started && now - s.since >= START_MS {
                s.started = true;
                out.push(Event::Started(app));
            }
        }
        self.0.retain(|&app, s| {
            if s.last == now {
                true
            } else if !s.started {
                false // a break before the start begins the count again
            } else if now - s.last >= END_MS {
                out.push(Event::Ended(app));
                false
            } else {
                true
            }
        });
        out
    }
}

fn check(what: &str, status: i32) -> Result<(), String> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("{what} failed (OSStatus {status})"))
    }
}

fn address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// # Safety
/// `T` must be the property's own type (a `u32`, an `i32`, or a CFString pointer the caller releases).
unsafe fn read<T>(object: AudioObjectID, selector: u32, out: &mut T) -> Result<(), String> {
    let addr = address(selector);
    let mut size = size_of::<T>() as u32;
    check(
        "read an audio process",
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&addr),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(out).cast::<c_void>(),
        ),
    )
}

/// Every Core Audio process object (macOS 14+).
fn processes() -> Result<Vec<AudioObjectID>, String> {
    let addr = address(kAudioHardwarePropertyProcessObjectList);
    let system = kAudioObjectSystemObject as AudioObjectID;
    unsafe {
        let mut size = 0u32;
        check(
            "list audio processes",
            AudioObjectGetPropertyDataSize(
                system,
                NonNull::from(&addr),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
            ),
        )?;
        let mut ids = vec![0 as AudioObjectID; size as usize / size_of::<AudioObjectID>()];
        if ids.is_empty() {
            return Ok(ids);
        }
        check(
            "list audio processes",
            AudioObjectGetPropertyData(
                system,
                NonNull::from(&addr),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::new_unchecked(ids.as_mut_ptr()).cast::<c_void>(),
            ),
        )?;
        ids.truncate(size as usize / size_of::<AudioObjectID>());
        Ok(ids)
    }
}

/// The bundle ids of the processes reading the microphone now, except this app's own capture.
pub fn mic_holders() -> Result<Vec<String>, String> {
    let me = std::process::id() as i32;
    let mut out = Vec::new();
    for p in processes()? {
        unsafe {
            let (mut input, mut pid) = (0u32, 0i32);
            // A process can exit between the list and these reads; skip it.
            if read(p, kAudioProcessPropertyIsRunningInput, &mut input).is_err()
                || input == 0
                || read(p, kAudioProcessPropertyPID, &mut pid).is_err()
                || pid == me
            {
                continue;
            }
            let mut s: *const CFString = std::ptr::null();
            if read(p, kAudioProcessPropertyBundleID, &mut s).is_ok() {
                if let Some(s) = NonNull::new(s as *mut CFString) {
                    out.push(CFRetained::from_raw(s).to_string());
                }
            }
        }
    }
    Ok(out)
}

/// Polls the microphone holders on its own thread and hands each start and end to `on_event`.
pub fn start(on_event: impl Fn(Event) + Send + 'static) {
    thread::spawn(move || {
        let t0 = Instant::now();
        let mut detector = Detector::default();
        loop {
            // ponytail: polling every 2 s; switch to Core Audio property listeners if it shows up in CPU use.
            // A failed read is not a release: skip the sample instead of ending a call.
            match mic_holders() {
                Ok(b) => {
                    let holding: Vec<&'static str> = b.iter().filter_map(|b| app_of(b)).collect();
                    for e in detector.tick(t0.elapsed().as_millis() as u64, &holding) {
                        on_event(e);
                    }
                }
                Err(e) => eprintln!("meeting detection: {e}"),
            }
            thread::sleep(POLL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use Event::*;

    #[test]
    fn maps_apps_and_their_helpers_only() {
        assert_eq!(app_of("us.zoom.xos"), Some("Zoom"));
        assert_eq!(app_of("com.google.Chrome.helper"), Some("Google Chrome"));
        assert_eq!(app_of("company.thebrowser.browser.helper"), Some("Arc"));
        assert_eq!(app_of("com.microsoft.teams2"), Some("Microsoft Teams"));
        assert_eq!(
            app_of("com.google.Chromium"),
            None,
            "a longer id is another app"
        );
        assert_eq!(app_of("com.apple.VoiceMemos"), None);
        assert_eq!(app_of(""), None);
    }

    #[test]
    fn starts_after_five_seconds_held_and_ends_after_fifteen_released() {
        let mut d = Detector::default();
        assert_eq!(d.tick(0, &["Zoom"]), vec![]);
        assert_eq!(d.tick(4_999, &["Zoom"]), vec![]);
        assert_eq!(d.tick(5_000, &["Zoom"]), vec![Started("Zoom")]);
        assert_eq!(d.tick(7_000, &["Zoom", "Zoom"]), vec![], "started once");
        // A 14 s release (muting, a device switch) is the same call.
        assert_eq!(d.tick(21_000, &[]), vec![]);
        assert_eq!(d.tick(21_999, &["Zoom"]), vec![]);
        assert_eq!(d.tick(36_998, &[]), vec![]);
        assert_eq!(d.tick(36_999, &[]), vec![Ended("Zoom")]);
        assert_eq!(d.tick(40_000, &[]), vec![], "ended once");
    }

    #[test]
    fn a_break_before_the_start_begins_the_count_again() {
        let mut d = Detector::default();
        d.tick(0, &["Slack"]);
        d.tick(4_000, &[]);
        assert_eq!(d.tick(6_000, &["Slack"]), vec![]);
        assert_eq!(d.tick(10_999, &["Slack"]), vec![]);
        assert_eq!(d.tick(11_000, &["Slack"]), vec![Started("Slack")]);
        assert_eq!(d.tick(20_000, &[]), vec![]);
        assert_eq!(d.tick(60_000, &[]), vec![Ended("Slack")]);
    }

    #[test]
    fn apps_are_tracked_apart() {
        let mut d = Detector::default();
        d.tick(0, &["Zoom", "Safari"]);
        assert_eq!(
            d.tick(5_000, &["Zoom", "Safari"]),
            vec![Started("Zoom"), Started("Safari")]
        );
        assert_eq!(d.tick(20_000, &["Zoom"]), vec![Ended("Safari")]);
    }

    #[test]
    fn lists_the_microphone_holders_without_failing() {
        // Runs against the real Core Audio of this Mac; this test process itself is not using the microphone.
        assert!(mic_holders().is_ok());
    }
}
