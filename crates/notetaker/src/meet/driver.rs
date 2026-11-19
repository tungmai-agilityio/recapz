//! The Meet `CallDriver`: joins through `MeetPage`, records the call audio only after admission
//! and the chat notice, and turns what the page shows into the events `Session` decides on.
use super::{JoinStep, MeetPage, PageState};
use crate::{
    browser::{Browser, Launch},
    capture::{self, AudioCapture},
    driver::{CallDriver, DriverError, DriverEvent},
    session::Event,
    speakers::Sample,
};
use std::{
    collections::VecDeque,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// How often the page is looked at (the participant and speaker sampling `Session` expects).
const POLL: Duration = Duration::from_millis(250);
/// No audio frame this long after admission: the capture is broken, so leave.
const NO_AUDIO: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
pub struct MeetConfig {
    pub launch: Launch,
    /// The chat message that tells the call it is being recorded and how to opt out.
    pub notice: String,
    /// When set, a failing page step or a refused join saves a screenshot and the page's buttons
    /// here (a private folder for this call), so a changed page or the refusal can be diagnosed.
    pub debug_dir: Option<std::path::PathBuf>,
}

enum Phase {
    Joining,
    /// Admitted and announced; `since` is when recording started (speaker sample times count from it).
    InCall {
        since: Instant,
        last_audio: Instant,
        chat_seen: usize,
    },
}

struct Run {
    // Field order matters on drop: the page and capture go before the browser that owns them.
    meet: MeetPage,
    capture: AudioCapture,
    _browser: Browser, // held so it is killed when the run ends
    phase: Phase,
    last_poll: Instant,
    /// Frames thrown away before recording started (tests check the consent ordering with it).
    dropped_frames: u64,
    /// The part of `dropped_frames` thrown away at the moment of admission, after the notice.
    dropped_at_admission: u64,
}

pub struct MeetDriver {
    cfg: MeetConfig,
    run: Option<Run>,
    pending: VecDeque<DriverEvent>,
}

/// Saves what the page shows into `dir` when one is set. A wedged browser must not delay the
/// failure being reported.
async fn save_page(meet: &MeetPage, dir: Option<&std::path::Path>) {
    if let Some(dir) = dir {
        let _ = tokio::time::timeout(Duration::from_secs(10), meet.snapshot(dir)).await;
    }
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl MeetDriver {
    pub fn new(cfg: MeetConfig) -> Self {
        Self {
            cfg,
            run: None,
            pending: VecDeque::new(),
        }
    }

    /// One look at the page; pushes the resulting events.
    async fn poll(&mut self) -> Result<(), DriverError> {
        let run = self.run.as_mut().ok_or(DriverError::Disconnected)?;
        run.last_poll = Instant::now();
        let at_ms = unix_ms();
        match &mut run.phase {
            Phase::Joining => match run.meet.step_join(&self.cfg.notice).await? {
                JoinStep::Waiting => self
                    .pending
                    .push_back(DriverEvent::Call(Event::Tick { at_ms })),
                JoinStep::Denied => {
                    // Meet says why on the page (a person declined, or the account may not join).
                    save_page(&run.meet, self.cfg.debug_dir.as_deref()).await;
                    self.pending.push_back(DriverEvent::Call(Event::Denied));
                }
                JoinStep::Admitted => {
                    // Everything in the log now is ours (the notice) or older: only later messages count.
                    let seen = run.meet.observe().await?.messages.len();
                    // Frames buffered during the lobby and while the notice was posted are not ours
                    // to keep: recording starts now.
                    while run.capture.frames.try_recv().is_ok() {
                        run.dropped_frames += 1;
                        run.dropped_at_admission += 1;
                    }
                    run.phase = Phase::InCall {
                        since: Instant::now(),
                        last_audio: Instant::now(),
                        chat_seen: seen,
                    };
                    self.pending.push_back(DriverEvent::Call(Event::Admitted));
                }
            },
            Phase::InCall {
                since,
                last_audio,
                chat_seen,
            } => {
                if last_audio.elapsed() >= NO_AUDIO {
                    return Err(DriverError::NoAudio);
                }
                let o = run.meet.observe().await?;
                match o.state {
                    PageState::Removed => self.pending.push_back(DriverEvent::Call(Event::Removed)),
                    PageState::Ended => self
                        .pending
                        .push_back(DriverEvent::Call(Event::CallEnded { at_ms })),
                    PageState::SignedOut => return Err(DriverError::SignedOut),
                    _ => {}
                }
                if let Some(count) = o.count {
                    self.pending
                        .push_back(DriverEvent::Call(Event::Participants { at_ms, count }));
                }
                if let Some(name) = o.speaker {
                    let at = since.elapsed().as_millis() as i64;
                    self.pending.push_back(DriverEvent::Speaker(Sample {
                        at_ms: at,
                        speaker: name,
                    }));
                }
                // The log can shrink (panel closed and reopened): count from what is there now.
                *chat_seen = (*chat_seen).min(o.messages.len());
                for m in o.messages.into_iter().skip(*chat_seen) {
                    *chat_seen += 1;
                    self.pending.push_back(DriverEvent::Call(Event::Chat {
                        text: m.text,
                        from_self: m.own,
                    }));
                }
                self.pending
                    .push_back(DriverEvent::Call(Event::Tick { at_ms }));
            }
        }
        Ok(())
    }
}

impl CallDriver for MeetDriver {
    async fn join(&mut self, url: &str) -> Result<(), DriverError> {
        let browser = Browser::launch(self.cfg.launch.clone())
            .await
            .map_err(|_| DriverError::Disconnected)?;
        let page = browser
            .inner
            .new_page("about:blank")
            .await
            .map_err(|_| DriverError::Disconnected)?;
        // Frames flow from the moment the page loads; they are dropped until admission.
        let capture = capture::start(&page)
            .await
            .map_err(|_| DriverError::Disconnected)?;
        let meet = MeetPage::new(page);
        meet.open(url).await?;
        self.run = Some(Run {
            meet,
            capture,
            _browser: browser,
            phase: Phase::Joining,
            last_poll: Instant::now()
                .checked_sub(POLL)
                .unwrap_or_else(Instant::now),
            dropped_frames: 0,
            dropped_at_admission: 0,
        });
        Ok(())
    }

    /// Not cancel-safe in the middle of a poll: a dropped call can post the notice twice. The
    /// loop awaits it to completion.
    async fn next(&mut self) -> Result<DriverEvent, DriverError> {
        loop {
            if let Some(e) = self.pending.pop_front() {
                return Ok(e);
            }
            let run = self.run.as_mut().ok_or(DriverError::Disconnected)?;
            let wait = POLL.saturating_sub(run.last_poll.elapsed());
            match &mut run.phase {
                Phase::Joining => {
                    // Nothing is kept before admission.
                    while run.capture.frames.try_recv().is_ok() {
                        run.dropped_frames += 1;
                    }
                    tokio::time::sleep(wait).await;
                }
                Phase::InCall { last_audio, .. } => {
                    tokio::select! {
                        frame = run.capture.frames.recv() => match frame {
                            Some(f) => {
                                *last_audio = Instant::now();
                                return Ok(DriverEvent::Audio(f));
                            }
                            None => return Err(DriverError::Disconnected),
                        },
                        _ = tokio::time::sleep(wait) => {}
                    }
                }
            }
            if run.last_poll.elapsed() >= POLL {
                if let Err(e) = self.poll().await {
                    if let (DriverError::UiChanged(_), Some(run)) = (&e, &self.run) {
                        save_page(&run.meet, self.cfg.debug_dir.as_deref()).await;
                    }
                    return Err(e);
                }
            }
        }
    }

    async fn leave(&mut self) {
        if let Some(run) = self.run.take() {
            // Counts only, so a call that recorded nothing says where the audio stopped.
            let stats =
                tokio::time::timeout(Duration::from_secs(3), run.meet.capture_stats()).await;
            let s = match stats {
                Ok(Some(s)) => s,
                _ => "stats unavailable".to_string(), // the page was already gone
            };
            tracing::info!(
                "audio capture: {s} dropped_before_admission={} dropped={}",
                run.dropped_at_admission,
                run.dropped_frames
            );
            let _ = run.meet.leave().await; // best effort; closing the browser ends it anyway
            drop(run);
        }
    }
}

/// Real Chromium against our own fixture page; run with `make test-browser`.
#[cfg(all(test, feature = "browser-tests"))]
mod browser_tests {
    use super::*;
    use crate::{browser::find_chromium, test_support::serve};
    use std::path::PathBuf;

    const FIXTURE: &str = include_str!("fixture.html");
    const NOTICE: &str = "Recapz notetaker is recording this call. Type 'opt out' to stop.";

    fn driver(scenario: &str) -> Option<(MeetDriver, String, PathBuf)> {
        if find_chromium(None).is_none() {
            println!("SKIPPED: no Chromium found");
            return None;
        }
        let work =
            std::env::temp_dir().join(format!("recapz-drv-{}-{scenario}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        let cfg = MeetConfig {
            launch: Launch {
                chromium: None,
                profile: None,
                work_dir: work.clone(),
                headless: true,
            },
            notice: NOTICE.into(),
            debug_dir: None,
        };
        Some((
            MeetDriver::new(cfg),
            format!("{}?s={scenario}", serve(FIXTURE)),
            work,
        ))
    }

    /// Joins and collects events until `stop` accepts one or the driver errors (that error is returned too).
    async fn collect(
        d: &mut MeetDriver,
        url: &str,
        stop: impl Fn(&DriverEvent) -> bool,
    ) -> (Vec<DriverEvent>, Option<DriverError>) {
        d.join(url).await.unwrap();
        let mut seen = Vec::new();
        let run = async {
            loop {
                match d.next().await {
                    Ok(e) => {
                        let done = stop(&e);
                        seen.push(e);
                        if done {
                            return None;
                        }
                    }
                    Err(e) => return Some(e),
                }
            }
        };
        let err = tokio::time::timeout(Duration::from_secs(30), run)
            .await
            .expect("timed out");
        (seen, err)
    }

    fn call(e: &DriverEvent) -> Option<&Event> {
        match e {
            DriverEvent::Call(c) => Some(c),
            _ => None,
        }
    }

    #[tokio::test]
    async fn reports_admission_participants_speaker_audio_and_then_removal() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((mut d, url, work)) = driver("removed") else {
            return;
        };
        let (seen, err) = collect(&mut d, &url, |e| matches!(call(e), Some(Event::Removed))).await;
        assert_eq!(err, None);
        let pos = |f: &dyn Fn(&DriverEvent) -> bool| seen.iter().position(f).unwrap();
        let admitted = pos(&|e| matches!(call(e), Some(Event::Admitted)));
        let removed = pos(&|e| matches!(call(e), Some(Event::Removed)));
        assert!(admitted < removed);
        assert!(seen
            .iter()
            .any(|e| matches!(call(e), Some(Event::Participants { count: 3, .. }))));
        assert!(seen
            .iter()
            .any(|e| matches!(e, DriverEvent::Speaker(s) if s.speaker == "Ann")));
        assert!(seen[admitted..]
            .iter()
            .any(|e| matches!(e, DriverEvent::Audio(a) if a.len() == 1600)));
        d.leave().await;
        assert!(
            !work.exists(),
            "leaving closes the browser and removes the profile copy"
        );
        d.leave().await; // twice is fine
    }

    #[tokio::test]
    async fn nothing_is_recorded_before_admission_and_the_lobby_ticks() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((mut d, url, _)) = driver("lobby_admit") else {
            return;
        };
        let (seen, err) = collect(&mut d, &url, |e| matches!(e, DriverEvent::Audio(_))).await;
        assert_eq!(err, None);
        let first_audio = seen.len() - 1;
        let admitted = seen
            .iter()
            .position(|e| matches!(call(e), Some(Event::Admitted)))
            .unwrap();
        assert!(admitted < first_audio, "no audio before admission");
        assert!(seen[..admitted]
            .iter()
            .all(|e| matches!(call(e), Some(Event::Tick { .. }))));
        assert!(admitted >= 2, "waited in the lobby");
        // Frames flowed during the lobby and the notice, and every one of them was thrown away:
        // the page produces one every 100 ms, so over a 1.2 s lobby there are many.
        let dropped = d.run.as_ref().unwrap().dropped_frames;
        assert!(dropped >= 5, "dropped {dropped}");
        d.leave().await;
    }

    #[tokio::test]
    async fn call_end_alone_and_a_members_opt_out_are_reported_and_the_notice_is_not() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((mut d, url, _)) = driver("opt_out") else {
            return;
        };
        let (seen, _) = collect(&mut d, &url, |e| {
            matches!(call(e), Some(Event::Chat { .. }))
        })
        .await;
        let chats: Vec<_> = seen
            .iter()
            .filter_map(call)
            .filter(|e| matches!(e, Event::Chat { .. }))
            .collect();
        assert_eq!(
            chats,
            [&Event::Chat {
                text: "Sam: Please opt out, thanks".into(),
                from_self: false
            }]
        );
        assert!(crate::optout::is_opt_out("Sam: Please opt out, thanks"));
        d.leave().await;

        let Some((mut d, url, _)) = driver("ended") else {
            return;
        };
        let (seen, _) = collect(&mut d, &url, |e| {
            matches!(call(e), Some(Event::CallEnded { .. }))
        })
        .await;
        assert!(seen
            .iter()
            .any(|e| matches!(call(e), Some(Event::CallEnded { .. }))));
        d.leave().await;

        let Some((mut d, url, _)) = driver("alone") else {
            return;
        };
        let (seen, _) = collect(&mut d, &url, |e| {
            matches!(call(e), Some(Event::Participants { .. }))
        })
        .await;
        assert!(matches!(
            call(seen.last().unwrap()),
            Some(Event::Participants { count: 1, .. })
        ));
        d.leave().await;
    }

    #[tokio::test]
    async fn a_chat_that_is_off_stops_the_call_before_any_recording() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((mut d, url, work)) = driver("chat_off") else {
            return;
        };
        let (seen, err) = collect(&mut d, &url, |_| false).await;
        assert_eq!(err, Some(DriverError::ChatOff));
        assert!(
            seen.iter()
                .all(|e| matches!(call(e), Some(Event::Tick { .. }))),
            "no Admitted event and no audio"
        );
        d.leave().await;
        assert!(!work.exists());
    }

    #[tokio::test]
    async fn a_changed_join_page_saves_a_screenshot_and_its_buttons_when_asked_to() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((mut d, url, work)) = driver("no_join_button") else {
            return;
        };
        let dump = work.with_extension("dump");
        let _ = std::fs::remove_dir_all(&dump);
        d.cfg.debug_dir = Some(dump.clone());
        d.join(&url).await.unwrap();
        d.run.as_mut().unwrap().meet.unknown_limit = Duration::from_secs(2);
        let err = loop {
            if let Err(e) = d.next().await {
                break e;
            }
        };
        assert_eq!(err, DriverError::UiChanged("join_button".into()));
        let page: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dump.join("page.json")).unwrap())
                .unwrap();
        let names: Vec<&str> = page["controls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["role"] == "button")
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["Enter"], "the buttons the page really has");
        assert!(page["text"].as_str().unwrap().contains("Enter"));
        let png = std::fs::read(dump.join("page.png")).unwrap();
        assert_eq!(&png[1..4], b"PNG", "a real screenshot");
        d.leave().await;
        let _ = std::fs::remove_dir_all(&dump);
    }

    #[tokio::test]
    async fn a_refused_join_saves_what_the_page_said_when_asked_to() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((mut d, url, work)) = driver("cannot_join") else {
            return;
        };
        let dump = work.with_extension("dump");
        let _ = std::fs::remove_dir_all(&dump);
        d.cfg.debug_dir = Some(dump.clone());
        let (seen, err) = collect(&mut d, &url, |e| matches!(call(e), Some(Event::Denied))).await;
        assert_eq!(err, None);
        assert!(matches!(call(seen.last().unwrap()), Some(Event::Denied)));
        let page: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dump.join("page.json")).unwrap())
                .unwrap();
        assert!(
            page["text"]
                .as_str()
                .unwrap()
                .contains("You can't join this video call"),
            "{page}"
        );
        let png = std::fs::read(dump.join("page.png")).unwrap();
        assert_eq!(&png[1..4], b"PNG", "a real screenshot");
        d.leave().await;
        let _ = std::fs::remove_dir_all(&dump);
    }

    #[tokio::test]
    async fn an_opt_out_is_seen_after_the_chat_log_was_cleared_and_from_a_name_starting_with_you() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((mut d, url, _)) = driver("chat_reopen") else {
            return;
        };
        let (seen, _) = collect(&mut d, &url, |e| {
            matches!(call(e), Some(Event::Chat { .. }))
        })
        .await;
        assert_eq!(
            seen.iter()
                .filter_map(call)
                .filter(|e| matches!(e, Event::Chat { .. }))
                .collect::<Vec<_>>(),
            [&Event::Chat {
                text: "Sam: opt out please".into(),
                from_self: false
            }]
        );
        d.leave().await;

        let Some((mut d, url, _)) = driver("name_you") else {
            return;
        };
        let (seen, _) = collect(&mut d, &url, |e| {
            matches!(call(e), Some(Event::Chat { .. }))
        })
        .await;
        assert_eq!(
            seen.iter()
                .filter_map(call)
                .filter(|e| matches!(e, Event::Chat { .. }))
                .collect::<Vec<_>>(),
            [&Event::Chat {
                text: "You Li: opt out".into(),
                from_self: false
            }],
            "not mistaken for the notetaker's own message"
        );
        d.leave().await;
    }

    #[tokio::test]
    async fn audio_that_arrived_while_the_notice_was_being_posted_is_not_kept() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        // `slow_chat` takes 0.8 s to show our message, so about 8 frames pile up meanwhile.
        let Some((mut d, url, _)) = driver("slow_chat") else {
            return;
        };
        let (seen, err) = collect(&mut d, &url, |e| matches!(e, DriverEvent::Audio(_))).await;
        assert_eq!(err, None);
        let admitted = seen
            .iter()
            .position(|e| matches!(call(e), Some(Event::Admitted)))
            .unwrap();
        assert!(
            admitted < seen.len() - 1,
            "the first audio frame comes after Admitted"
        );
        let dropped = d.run.as_ref().unwrap().dropped_at_admission;
        assert!(
            dropped >= 5,
            "only {dropped} buffered frames were thrown away at admission"
        );
        d.leave().await;
    }
}
