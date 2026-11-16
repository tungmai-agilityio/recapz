//! Joining a Meet call through its web page and announcing the notetaker in chat. The page lookups
//! live in `meet/page.js`; this side drives them and turns failures into `DriverError`s.
//! `driver` builds the `CallDriver` for Meet on top of it.
use crate::driver::DriverError;
use chromiumoxide::{cdp::browser_protocol::input::InsertTextParams, Page};
use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};

mod driver;
pub use driver::{MeetConfig, MeetDriver};

const SCRIPT: &str = include_str!("meet/page.js");
const POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageState {
    SignedOut,
    Prejoin,
    Lobby,
    Admitted,
    Denied,
    /// A participant removed the notetaker.
    Removed,
    /// The call is over (ended, or the notetaker is no longer in it).
    Ended,
    /// None of the known phases: the page loads, or it has changed.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joined {
    /// Admitted, and the notice is in the chat. Capture may start.
    Admitted,
    Denied,
    /// Still not admitted when `lobby_limit` ran out.
    LobbyTimeout,
}

/// What one pass of the joining loop found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinStep {
    Waiting,
    /// Admitted and the notice is in the chat.
    Admitted,
    /// Refused, removed or the call is gone before admission.
    Denied,
}

pub(crate) struct ChatLine {
    pub text: String,
    /// Posted by the notetaker itself.
    pub own: bool,
}

pub(crate) struct Observation {
    pub state: PageState,
    pub count: Option<u32>,
    pub speaker: Option<String>,
    pub messages: Vec<ChatLine>,
}

fn parse_state(s: Option<&str>) -> PageState {
    match s {
        Some("signed_out") => PageState::SignedOut,
        Some("prejoin") => PageState::Prejoin,
        Some("lobby") => PageState::Lobby,
        Some("admitted") => PageState::Admitted,
        Some("denied") => PageState::Denied,
        Some("removed") => PageState::Removed,
        Some("ended") => PageState::Ended,
        _ => PageState::Unknown,
    }
}

/// The browser or page call failed: keep why in the log (the call's own script names and the
/// DevTools error, never anything said in the call) before it becomes the one `Disconnected`.
fn dropped(what: &str, e: impl std::fmt::Display) -> DriverError {
    tracing::warn!("page call failed ({what}): {e}");
    DriverError::Disconnected
}

pub struct MeetPage {
    page: Page,
    /// How long no known phase may show before the page counts as changed.
    pub unknown_limit: Duration,
    asked: Option<Instant>,
    unknown_since: Option<Instant>,
}

impl MeetPage {
    pub fn new(page: Page) -> Self {
        Self {
            page,
            unknown_limit: Duration::from_secs(20),
            asked: None,
            unknown_since: None,
        }
    }

    /// Opens the call link, with the page script installed before the call page's own scripts.
    pub async fn open(&self, url: &str) -> Result<(), DriverError> {
        self.page
            .evaluate_on_new_document(SCRIPT)
            .await
            .map_err(|e| dropped("evaluate_on_new_document", e))?;
        self.page.goto(url).await.map_err(|e| dropped("goto", e))?;
        Ok(())
    }

    /// Saves what the page shows (a screenshot and its buttons) into `dir`, for diagnosing a changed
    /// page. Best effort: the call is already failing. The page can show names, so `dir` is private.
    pub async fn snapshot(&self, dir: &Path) {
        use chromiumoxide::page::ScreenshotParams;
        use std::os::unix::fs::DirBuilderExt;
        if std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .is_err()
        {
            return;
        }
        if let Ok(Value::String(json)) = self.eval("window.__recapzMeet?.snapshot()").await {
            let _ = std::fs::write(dir.join("page.json"), json);
        }
        let _ = self
            .page
            .save_screenshot(ScreenshotParams::builder().build(), dir.join("page.png"))
            .await;
    }

    async fn eval(&self, expr: &str) -> Result<Value, DriverError> {
        let v = self
            .page
            .evaluate(expr)
            .await
            .map_err(|e| dropped(expr, e))?;
        Ok(v.into_value::<Value>().unwrap_or(Value::Null))
    }

    /// Runs a page action that answers `{ok, step}`; a failed step is a changed page.
    async fn act(&self, call: &str) -> Result<(), DriverError> {
        let v = self.eval(&format!("window.__recapzMeet?.{call}")).await?;
        let v: Value = match v {
            Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::Null),
            other => other,
        };
        if v["ok"] == true {
            return Ok(());
        }
        match v["step"].as_str() {
            Some("chat_disabled") => Err(DriverError::ChatOff),
            Some(step) => Err(DriverError::UiChanged(step.to_string())),
            None => Err(DriverError::UiChanged("page_script".to_string())),
        }
    }

    /// One round trip with everything the follow loop needs.
    pub(crate) async fn observe(&self) -> Result<Observation, DriverError> {
        let v = self.eval("window.__recapzMeet?.observe() ?? null").await?;
        let v: Value = match v {
            Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::Null),
            _ => Value::Null,
        };
        if v.is_null() {
            return Err(DriverError::UiChanged("page_script".to_string()));
        }
        Ok(Observation {
            state: parse_state(v["state"].as_str()),
            count: v["count"].as_u64().map(|c| c as u32),
            speaker: v["speaker"].as_str().map(str::to_string),
            messages: v["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| ChatLine {
                    text: m["text"].as_str().unwrap_or("").to_string(),
                    own: m["self"] == true,
                })
                .collect(),
        })
    }

    /// Clicks the leave button; the caller closes the browser either way.
    pub(crate) async fn leave(&self) -> Result<(), DriverError> {
        self.act("leave()").await
    }

    pub async fn state(&self) -> Result<PageState, DriverError> {
        let v = self
            .eval("window.__recapzMeet?.state() ?? 'unknown'")
            .await?;
        Ok(parse_state(v.as_str()))
    }

    /// Posts `notice` in the call chat and checks it appears. `ChatOff` when chat is disabled.
    pub async fn announce(&self, notice: &str) -> Result<(), DriverError> {
        if notice.trim().is_empty() {
            return Err(DriverError::UiChanged("notice_empty".to_string()));
        }
        let prefix = serde_json::to_string(&notice.chars().take(24).collect::<String>()).unwrap();
        self.act("openChat()").await?;
        // The panel is drawn a moment after the click: wait for its input before using it.
        let start = Instant::now();
        loop {
            match self.act("focusChatInput()").await {
                Err(DriverError::UiChanged(s)) if s == "chat_input" => {
                    if start.elapsed() >= Duration::from_secs(3) {
                        return Err(DriverError::UiChanged(s));
                    }
                    tokio::time::sleep(POLL).await;
                }
                other => break other?,
            }
        }
        // Someone else's message, or an earlier notice, must not count as ours.
        let before = self
            .eval(&format!("window.__recapzMeet.chatCount({prefix})"))
            .await?;
        self.page
            .execute(InsertTextParams::new(notice))
            .await
            .map_err(|e| dropped("insert_text", e))?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            // The send button turns on a moment after the text arrives, and a click before that is
            // lost. `clickSend` does nothing while it is off or the box is empty, so repeating it
            // until the notice shows cannot post it twice.
            self.act("clickSend()").await?;
            let now = self
                .eval(&format!("window.__recapzMeet.chatCount({prefix})"))
                .await?;
            if now.as_u64() > before.as_u64() {
                return Ok(());
            }
            tokio::time::sleep(POLL).await;
        }
        Err(DriverError::UiChanged("chat_send".to_string()))
    }

    /// One pass of joining: turns devices off and asks to join once, then reports whether the
    /// notetaker is let in (with the notice posted), refused, or still waiting. When the notice
    /// cannot be posted the error says why and the caller leaves without recording.
    pub async fn step_join(&mut self, notice: &str) -> Result<JoinStep, DriverError> {
        match self.state().await? {
            PageState::SignedOut => return Err(DriverError::SignedOut),
            PageState::Denied | PageState::Removed | PageState::Ended => {
                return Ok(JoinStep::Denied)
            }
            PageState::Admitted => {
                self.announce(notice).await?;
                return Ok(JoinStep::Admitted);
            }
            PageState::Prejoin if self.asked.is_none() => {
                self.act("prepare()").await?;
                self.act("join()").await?;
                self.asked = Some(Instant::now());
            }
            // Still on the join page after asking: the click did nothing, so the page changed.
            PageState::Prejoin
                if self
                    .asked
                    .is_some_and(|t| t.elapsed() >= self.unknown_limit) =>
            {
                return Err(DriverError::UiChanged("join_button".to_string()));
            }
            PageState::Unknown => {
                let since = *self.unknown_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= self.unknown_limit {
                    // Before asking, the join button is the missing piece; after, the lobby text.
                    let step = if self.asked.is_some() {
                        "lobby_state"
                    } else {
                        "join_button"
                    };
                    return Err(DriverError::UiChanged(step.to_string()));
                }
                return Ok(JoinStep::Waiting);
            }
            PageState::Prejoin | PageState::Lobby => {}
        }
        self.unknown_since = None;
        Ok(JoinStep::Waiting)
    }

    /// Opens `url` and joins, waiting to be let in for at most `lobby_limit`.
    pub async fn join(
        &mut self,
        url: &str,
        notice: &str,
        lobby_limit: Duration,
    ) -> Result<Joined, DriverError> {
        self.open(url).await?;
        let start = Instant::now();
        loop {
            match self.step_join(notice).await? {
                JoinStep::Admitted => return Ok(Joined::Admitted),
                JoinStep::Denied => return Ok(Joined::Denied),
                JoinStep::Waiting if start.elapsed() >= lobby_limit => {
                    return Ok(Joined::LobbyTimeout)
                }
                JoinStep::Waiting => tokio::time::sleep(POLL).await,
            }
        }
    }
}

/// Real Chromium against our own fixture page; run with `make test-browser`.
#[cfg(all(test, feature = "browser-tests"))]
mod browser_tests {
    use super::*;
    use crate::{
        browser::{find_chromium, Browser, Launch},
        test_support::serve,
    };

    const FIXTURE: &str = include_str!("meet/fixture.html");

    /// Joins the fixture in `scenario`; returns the result and the page for follow-up checks.
    async fn run(
        scenario: &str,
        lobby_limit: Duration,
    ) -> Option<(Result<Joined, DriverError>, MeetPage, Browser)> {
        if find_chromium(None).is_none() {
            println!("SKIPPED: no Chromium found");
            return None;
        }
        let work =
            std::env::temp_dir().join(format!("recapz-meet-{}-{scenario}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        let b = Browser::launch(Launch {
            chromium: None,
            profile: None,
            work_dir: work,
            headless: true,
        })
        .await
        .unwrap();
        let mut meet = MeetPage::new(b.inner.new_page("about:blank").await.unwrap());
        meet.unknown_limit = Duration::from_secs(2);
        let url = format!("{}?s={scenario}", serve(FIXTURE));
        let r = meet
            .join(
                &url,
                "Recapz notetaker is recording this call. Type 'opt out' to stop.",
                lobby_limit,
            )
            .await;
        Some((r, meet, b))
    }

    async fn flag(meet: &MeetPage, name: &str) -> bool {
        meet.eval(&format!("window.{name} === true")).await.unwrap() == true
    }

    #[tokio::test]
    async fn admitted_at_once_turns_devices_off_and_posts_the_notice_once() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((r, meet, _b)) = run("admit", Duration::from_secs(30)).await else {
            return;
        };
        assert_eq!(r, Ok(Joined::Admitted));
        assert!(
            flag(&meet, "camOff").await && flag(&meet, "micOff").await,
            "camera and microphone off"
        );
        let log = meet
            .eval("[...document.querySelectorAll('ul[aria-label=\"Chat messages\"] li')].map(l => l.textContent)")
            .await
            .unwrap();
        assert_eq!(
            log,
            serde_json::json!([
                "You: Recapz notetaker is recording this call. Type 'opt out' to stop."
            ])
        );
    }

    #[tokio::test]
    async fn waits_in_the_lobby_then_posts_the_notice_after_admission() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((r, meet, _b)) = run("lobby_admit", Duration::from_secs(30)).await else {
            return;
        };
        assert_eq!(r, Ok(Joined::Admitted));
        assert_eq!(meet.state().await, Ok(PageState::Admitted));
    }

    #[tokio::test]
    async fn vietnamese_labels_and_lobby_text_are_recognised() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((r, meet, _b)) = run("vi_lobby_admit", Duration::from_secs(30)).await else {
            return;
        };
        assert_eq!(r, Ok(Joined::Admitted));
        assert_eq!(meet.state().await, Ok(PageState::Admitted));
    }

    #[tokio::test]
    async fn the_english_host_lobby_text_is_still_the_lobby() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((r, meet, _b)) = run("lobby_host", Duration::from_secs(30)).await else {
            return;
        };
        assert_eq!(r, Ok(Joined::Admitted));
        assert_eq!(meet.state().await, Ok(PageState::Admitted));
    }

    #[tokio::test]
    async fn a_send_button_that_turns_on_late_still_posts_the_notice() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((r, meet, _b)) = run("send_late", Duration::from_secs(30)).await else {
            return;
        };
        assert_eq!(r, Ok(Joined::Admitted));
        // Exactly one notice: retrying the click must not post it twice.
        let count = meet
            .eval("window.__recapzMeet.chatCount('Recapz notetaker is reco')")
            .await;
        assert_eq!(count, Ok(serde_json::json!(1)));
    }

    #[tokio::test]
    async fn the_notice_is_confirmed_even_when_the_chat_log_has_no_label() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let Some((r, meet, _b)) = run("log_unlabelled", Duration::from_secs(30)).await else {
            return;
        };
        assert_eq!(r, Ok(Joined::Admitted));
        assert_eq!(meet.state().await, Ok(PageState::Admitted));
    }

    #[tokio::test]
    async fn the_end_states_are_reported_as_what_they_are() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        let cases: [(&str, Duration, Result<Joined, DriverError>); 8] = [
            ("denied", Duration::from_secs(30), Ok(Joined::Denied)),
            (
                "lobby_stuck",
                Duration::from_secs(2),
                Ok(Joined::LobbyTimeout),
            ),
            (
                "signed_out",
                Duration::from_secs(30),
                Err(DriverError::SignedOut),
            ),
            (
                "chat_off",
                Duration::from_secs(30),
                Err(DriverError::ChatOff),
            ),
            (
                "chat_send_broken",
                Duration::from_secs(30),
                Err(DriverError::UiChanged("chat_send".into())),
            ),
            (
                "no_join_button",
                Duration::from_secs(30),
                Err(DriverError::UiChanged("join_button".into())),
            ),
            (
                "join_noop",
                Duration::from_secs(30),
                Err(DriverError::UiChanged("join_button".into())),
            ),
            (
                "vanishes",
                Duration::from_secs(30),
                Err(DriverError::UiChanged("lobby_state".into())),
            ),
        ];
        for (scenario, limit, want) in cases {
            let Some((got, ..)) = run(scenario, limit).await else {
                return;
            };
            assert_eq!(got, want, "{scenario}");
        }
    }

    #[tokio::test]
    async fn another_persons_matching_message_does_not_count_and_an_empty_notice_is_refused() {
        let _one_at_a_time = crate::test_support::exclusive().await;
        // `chat_echo` shows a message that already starts like our notice.
        let Some((r, meet, _b)) = run("chat_echo", Duration::from_secs(30)).await else {
            return;
        };
        assert_eq!(
            r,
            Ok(Joined::Admitted),
            "our own post raises the count above the echo"
        );
        let n = meet
            .eval("document.querySelectorAll('ul[aria-label=\"Chat messages\"] li').length")
            .await
            .unwrap();
        assert_eq!(n, 2, "the earlier message and exactly one notice of ours");
        assert_eq!(
            meet.announce("  ").await,
            Err(DriverError::UiChanged("notice_empty".into()))
        );
    }
}
