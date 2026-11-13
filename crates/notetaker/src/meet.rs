//! Joining a Meet call through its web page and announcing the notetaker in chat. The page lookups
//! live in `meet/page.js`; this side drives them and turns failures into `DriverError`s.
//! Following the call (removal, end, participants, speakers) is the next step.
use crate::driver::DriverError;
use chromiumoxide::{cdp::browser_protocol::input::InsertTextParams, Page};
use serde_json::Value;
use std::time::{Duration, Instant};

const SCRIPT: &str = include_str!("meet/page.js");
const POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageState {
    SignedOut,
    Prejoin,
    Lobby,
    Admitted,
    Denied,
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

pub struct MeetPage {
    page: Page,
    /// How long no known phase may show before the page counts as changed.
    pub unknown_limit: Duration,
}

impl MeetPage {
    pub fn new(page: Page) -> Self {
        Self {
            page,
            unknown_limit: Duration::from_secs(20),
        }
    }

    /// Opens the call link, with the page script installed before the call page's own scripts.
    pub async fn open(&self, url: &str) -> Result<(), DriverError> {
        self.page
            .evaluate_on_new_document(SCRIPT)
            .await
            .map_err(|_| DriverError::Disconnected)?;
        self.page
            .goto(url)
            .await
            .map_err(|_| DriverError::Disconnected)?;
        Ok(())
    }

    async fn eval(&self, expr: &str) -> Result<Value, DriverError> {
        let v = self
            .page
            .evaluate(expr)
            .await
            .map_err(|_| DriverError::Disconnected)?;
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

    pub async fn state(&self) -> Result<PageState, DriverError> {
        let v = self
            .eval("window.__recapzMeet?.state() ?? 'unknown'")
            .await?;
        Ok(match v.as_str() {
            Some("signed_out") => PageState::SignedOut,
            Some("prejoin") => PageState::Prejoin,
            Some("lobby") => PageState::Lobby,
            Some("admitted") => PageState::Admitted,
            Some("denied") => PageState::Denied,
            _ => PageState::Unknown,
        })
    }

    /// Posts `notice` in the call chat and checks it appears. `ChatOff` when chat is disabled.
    pub async fn announce(&self, notice: &str) -> Result<(), DriverError> {
        if notice.trim().is_empty() {
            return Err(DriverError::UiChanged("notice_empty".to_string()));
        }
        let prefix = serde_json::to_string(&notice.chars().take(24).collect::<String>()).unwrap();
        self.act("openChat()").await?;
        // Someone else's message, or an earlier notice, must not count as ours.
        let before = self
            .eval(&format!("window.__recapzMeet.chatCount({prefix})"))
            .await?;
        self.act("focusChatInput()").await?;
        self.page
            .execute(InsertTextParams::new(notice))
            .await
            .map_err(|_| DriverError::Disconnected)?;
        self.act("clickSend()").await?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
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

    /// Opens `url`, turns camera and microphone off, asks to join, waits to be let in (at most
    /// `lobby_limit`) and posts `notice`. When the notice cannot be posted the call is not
    /// recorded: the error says why and the caller leaves.
    pub async fn join(
        &self,
        url: &str,
        notice: &str,
        lobby_limit: Duration,
    ) -> Result<Joined, DriverError> {
        self.open(url).await?;
        let (start, mut unknown_since, mut asked) =
            (Instant::now(), None::<Instant>, None::<Instant>);
        loop {
            match self.state().await? {
                PageState::SignedOut => return Err(DriverError::SignedOut),
                PageState::Denied => return Ok(Joined::Denied),
                PageState::Admitted => {
                    self.announce(notice).await?;
                    return Ok(Joined::Admitted);
                }
                PageState::Prejoin if asked.is_none() => {
                    self.act("prepare()").await?;
                    self.act("join()").await?;
                    asked = Some(Instant::now());
                }
                // Still on the join page after asking: the click did nothing, so the page changed.
                PageState::Prejoin if asked.is_some_and(|t| t.elapsed() >= self.unknown_limit) => {
                    return Err(DriverError::UiChanged("join_button".to_string()));
                }
                PageState::Unknown => {
                    let since = *unknown_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= self.unknown_limit {
                        // Before asking, the join button is the missing piece; after, the lobby text.
                        let step = if asked.is_some() {
                            "lobby_state"
                        } else {
                            "join_button"
                        };
                        return Err(DriverError::UiChanged(step.to_string()));
                    }
                }
                PageState::Prejoin | PageState::Lobby => {}
            }
            if !matches!(self.state().await?, PageState::Unknown) {
                unknown_since = None;
            }
            if start.elapsed() >= lobby_limit {
                return Ok(Joined::LobbyTimeout);
            }
            tokio::time::sleep(POLL).await;
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
        let Some((r, meet, _b)) = run("admit", Duration::from_secs(30)).await else {
            return;
        };
        assert_eq!(r, Ok(Joined::Admitted));
        assert!(
            flag(&meet, "camOff").await && flag(&meet, "micOff").await,
            "camera and microphone off"
        );
        let log = meet
            .eval("[...document.querySelectorAll('ul li')].map(l => l.textContent)")
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
        let Some((r, meet, _b)) = run("lobby_admit", Duration::from_secs(30)).await else {
            return;
        };
        assert_eq!(r, Ok(Joined::Admitted));
        assert_eq!(meet.state().await, Ok(PageState::Admitted));
    }

    #[tokio::test]
    async fn the_end_states_are_reported_as_what_they_are() {
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
            .eval("document.querySelectorAll('ul li').length")
            .await
            .unwrap();
        assert_eq!(n, 2, "the earlier message and exactly one notice of ours");
        assert_eq!(
            meet.announce("  ").await,
            Err(DriverError::UiChanged("notice_empty".into()))
        );
    }
}
