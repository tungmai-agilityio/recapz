//! The seam between the notetaker loop and a real call. A driver joins a call, announces the
//! notetaker in chat, and reports what happens; the loop feeds `Event`s to `Session` and carries
//! out the result. `ScriptedDriver` replays a fixed script so the loop is tested without a browser.
use crate::{session::Event, speakers::Sample};
use std::collections::VecDeque;

/// One thing a driver observed in the call.
#[derive(Debug, Clone, PartialEq)]
pub enum DriverEvent {
    /// A rule input for `Session` (admitted, participants, chat, ended, removed, denied, tick).
    Call(Event),
    /// 16 kHz mono 16-bit samples, about 100 ms per frame.
    Audio(Vec<i16>),
    /// Who was marked as speaking (about every 250 ms). Names are never logged.
    Speaker(Sample),
    /// The chat notice could not be posted and the notetaker records anyway (`require_notice` off).
    /// `step` is the failing page step, or `chat_off`.
    NoticeMissing { step: String },
}

/// Why a driver could not continue. The loop maps these to the meeting's failure message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverError {
    /// The join notice could not be posted, so the notetaker must not record.
    ChatOff,
    /// A page step failed to find its element; the string names the step.
    UiChanged(String),
    SignedOut,
    /// The browser crashed or the DevTools connection dropped.
    Disconnected,
    /// No audio frame for too long while admitted.
    NoAudio,
}

#[allow(async_fn_in_trait)] // only used with concrete types, never `dyn`
pub trait CallDriver {
    /// Start joining `url` (a normalised meeting link). Returns once the join was requested; the
    /// lobby and admission arrive as events. After admission the driver posts the chat notice
    /// before it reports any audio, and returns `ChatOff` from `next` if it cannot.
    async fn join(&mut self, url: &str) -> Result<(), DriverError>;
    /// The next observation, waiting if there is none yet.
    async fn next(&mut self) -> Result<DriverEvent, DriverError>;
    /// Leave the call and release the browser. Safe to call more than once.
    async fn leave(&mut self);
}

/// Replays `script` in order, then reports `Disconnected`.
#[derive(Debug, Default)]
pub struct ScriptedDriver {
    script: VecDeque<Result<DriverEvent, DriverError>>,
    pub joined: Option<String>,
    pub left: u32,
}

impl ScriptedDriver {
    pub fn new(script: impl IntoIterator<Item = Result<DriverEvent, DriverError>>) -> Self {
        Self {
            script: script.into_iter().collect(),
            ..Self::default()
        }
    }
}

impl CallDriver for ScriptedDriver {
    async fn join(&mut self, url: &str) -> Result<(), DriverError> {
        self.joined = Some(url.to_string());
        Ok(())
    }

    async fn next(&mut self) -> Result<DriverEvent, DriverError> {
        self.script
            .pop_front()
            .unwrap_or(Err(DriverError::Disconnected))
    }

    async fn leave(&mut self) {
        self.left += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Action, LeaveReason, Session};
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Waker},
    };

    /// The fake never waits, so one poll finishes every future.
    fn block_on<F: Future>(f: F) -> F::Output {
        match pin!(f).poll(&mut Context::from_waker(Waker::noop())) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("scripted driver should never be pending"),
        }
    }

    fn call(e: Event) -> Result<DriverEvent, DriverError> {
        Ok(DriverEvent::Call(e))
    }

    #[test]
    fn replays_events_audio_and_speaker_samples_in_order_then_disconnects() {
        let sample = Sample {
            at_ms: 250,
            speaker: "A".into(),
        };
        let mut d = ScriptedDriver::new([
            call(Event::Admitted),
            Ok(DriverEvent::Audio(vec![1, -2, 3])),
            Ok(DriverEvent::Speaker(sample.clone())),
        ]);
        block_on(d.join("https://meet.example/abc-defg-hij")).unwrap();
        assert_eq!(
            d.joined.as_deref(),
            Some("https://meet.example/abc-defg-hij")
        );
        assert_eq!(block_on(d.next()), call(Event::Admitted));
        assert_eq!(block_on(d.next()), Ok(DriverEvent::Audio(vec![1, -2, 3])));
        assert_eq!(block_on(d.next()), Ok(DriverEvent::Speaker(sample)));
        assert_eq!(block_on(d.next()), Err(DriverError::Disconnected));
        block_on(d.leave());
        block_on(d.leave());
        assert_eq!(d.left, 2);
    }

    #[test]
    fn a_scripted_error_comes_back_as_is() {
        let mut d = ScriptedDriver::new([Err(DriverError::ChatOff)]);
        assert_eq!(block_on(d.next()), Err(DriverError::ChatOff));
    }

    #[test]
    fn driving_a_session_from_the_script_leaves_on_removal() {
        let mut d = ScriptedDriver::new([
            call(Event::Admitted),
            Ok(DriverEvent::Audio(vec![0; 4])),
            call(Event::Removed),
            call(Event::CallEnded { at_ms: 9 }),
        ]);
        let mut session = Session::new(0);
        let (mut frames, mut leave) = (0, None);
        while let Ok(ev) = block_on(d.next()) {
            match ev {
                DriverEvent::Audio(_) => frames += 1,
                DriverEvent::Call(e) => {
                    if let Action::Leave(r) = session.on_event(e) {
                        leave = Some(r);
                        break;
                    }
                }
                DriverEvent::Speaker(_) | DriverEvent::NoticeMissing { .. } => {}
            }
        }
        assert_eq!(frames, 1);
        assert_eq!(leave, Some(LeaveReason::Removed));
    }
}
