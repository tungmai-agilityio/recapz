//! Decides what the notetaker does next from call events. No browser here: a driver feeds events in
//! and carries out the returned action, so the rules can be tested without a real call.
use crate::optout::is_opt_out;

pub const LOBBY_TIMEOUT_MS: i64 = 10 * 60 * 1000;
pub const ALONE_TIMEOUT_MS: i64 = 2 * 60 * 1000;
/// Default hard cap on a call (`max_call_minutes`, 240).
pub const MAX_CALL_MS: i64 = 240 * 60 * 1000;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The notetaker was let in.
    Admitted,
    /// Participant count (the notetaker included). The driver must sample this regularly (about
    /// every 250 ms); the alone timeout only starts from a sample showing nobody else.
    Participants {
        at_ms: i64,
        count: u32,
    },
    /// A chat message. `from_self` marks the notetaker's own messages (its join notice may mention
    /// how to opt out), which must never count as an opt-out.
    Chat {
        text: String,
        from_self: bool,
    },
    CallEnded {
        at_ms: i64,
    },
    /// The join was refused. Only meaningful before admission; ignored after.
    Denied,
    /// A participant removed the notetaker. Only meaningful after admission; ignored before.
    Removed,
    /// Periodic clock tick so timeouts fire when nothing else happens.
    Tick {
        at_ms: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveReason {
    LobbyTimeout,
    Alone,
    CallEnded,
    /// The host or a participant refused the join.
    Denied,
    /// A participant removed the notetaker mid-call. The audio so far is kept and transcribed.
    Removed,
    /// The call ran past the hard cap. The audio is kept and transcribed.
    MaxDuration,
    /// Someone asked not to be recorded: the recording, transcript and report must be deleted.
    OptOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Continue,
    Leave(LeaveReason),
}

#[derive(Debug)]
pub struct Session {
    started_ms: i64,
    admitted: bool,
    max_call_ms: i64,
    alone_since: Option<i64>,
    done: Option<LeaveReason>,
}

impl Session {
    /// `started_ms` is when the notetaker began trying to join.
    pub fn new(started_ms: i64) -> Self {
        Self {
            started_ms,
            admitted: false,
            max_call_ms: MAX_CALL_MS,
            alone_since: None,
            done: None,
        }
    }

    /// Overrides the hard cap on call length, measured from `started_ms`.
    pub fn with_max_call_ms(mut self, max_call_ms: i64) -> Self {
        self.max_call_ms = max_call_ms;
        self
    }

    /// Once a leave decision is made it stays; later events do not change it.
    pub fn on_event(&mut self, e: Event) -> Action {
        if let Some(r) = self.done {
            return Action::Leave(r);
        }
        let leave = match e {
            Event::Admitted => {
                self.admitted = true;
                self.alone_since = None; // lobby samples say nothing about the call itself
                None
            }
            Event::Chat {
                text,
                from_self: false,
            } if is_opt_out(&text) => Some(LeaveReason::OptOut),
            Event::Chat { .. } => None,
            Event::CallEnded { .. } => Some(LeaveReason::CallEnded),
            Event::Denied => (!self.admitted).then_some(LeaveReason::Denied),
            Event::Removed => self.admitted.then_some(LeaveReason::Removed),
            Event::Participants { at_ms, count } => {
                if !self.admitted {
                    // Ignored until admitted; the lobby timeout covers that time.
                } else if count <= 1 {
                    self.alone_since.get_or_insert(at_ms);
                } else {
                    self.alone_since = None;
                }
                self.timeouts(at_ms)
            }
            Event::Tick { at_ms } => self.timeouts(at_ms),
        };
        match leave {
            Some(r) => {
                self.done = Some(r);
                Action::Leave(r)
            }
            None => Action::Continue,
        }
    }

    fn timeouts(&self, now: i64) -> Option<LeaveReason> {
        // ponytail: the cap runs from the join attempt, so lobby time counts; fine at 240 min.
        if self.admitted && now - self.started_ms >= self.max_call_ms {
            return Some(LeaveReason::MaxDuration);
        }
        if !self.admitted {
            return (now - self.started_ms >= LOBBY_TIMEOUT_MS)
                .then_some(LeaveReason::LobbyTimeout);
        }
        match self.alone_since {
            Some(t) if now - t >= ALONE_TIMEOUT_MS => Some(LeaveReason::Alone),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(text: &str) -> Event {
        Event::Chat {
            text: text.into(),
            from_self: false,
        }
    }

    fn admitted() -> Session {
        let mut s = Session::new(0);
        s.on_event(Event::Admitted);
        s
    }

    #[test]
    fn opt_out_in_chat_leaves_even_before_admission() {
        let mut s = Session::new(0);
        assert_eq!(s.on_event(chat("hello")), Action::Continue);
        assert_eq!(
            s.on_event(chat("Opt out please")),
            Action::Leave(LeaveReason::OptOut)
        );
        // The decision sticks.
        assert_eq!(
            s.on_event(Event::Participants {
                at_ms: 30,
                count: 5
            }),
            Action::Leave(LeaveReason::OptOut)
        );
    }

    #[test]
    fn lobby_wait_ends_after_ten_minutes_unless_admitted() {
        let mut s = Session::new(1_000);
        assert_eq!(
            s.on_event(Event::Tick {
                at_ms: 1_000 + LOBBY_TIMEOUT_MS - 1
            }),
            Action::Continue
        );
        assert_eq!(
            s.on_event(Event::Tick {
                at_ms: 1_000 + LOBBY_TIMEOUT_MS
            }),
            Action::Leave(LeaveReason::LobbyTimeout)
        );

        let mut s = Session::new(0);
        s.on_event(Event::Admitted);
        assert_eq!(
            s.on_event(Event::Tick {
                at_ms: LOBBY_TIMEOUT_MS * 2
            }),
            Action::Continue,
            "lobby timeout no longer applies"
        );
    }

    #[test]
    fn leaves_after_two_minutes_alone_and_company_resets_the_clock() {
        let mut s = Session::new(0);
        s.on_event(Event::Admitted);
        assert_eq!(
            s.on_event(Event::Participants {
                at_ms: 1_000,
                count: 1
            }),
            Action::Continue
        );
        assert_eq!(
            s.on_event(Event::Participants {
                at_ms: 1_000 + ALONE_TIMEOUT_MS - 1,
                count: 3
            }),
            Action::Continue
        );
        // Alone again from a later time: the first stretch must not count.
        assert_eq!(
            s.on_event(Event::Participants {
                at_ms: 200_000,
                count: 1
            }),
            Action::Continue
        );
        assert_eq!(
            s.on_event(Event::Tick {
                at_ms: 200_000 + ALONE_TIMEOUT_MS - 1
            }),
            Action::Continue
        );
        assert_eq!(
            s.on_event(Event::Tick {
                at_ms: 200_000 + ALONE_TIMEOUT_MS
            }),
            Action::Leave(LeaveReason::Alone)
        );
    }

    #[test]
    fn call_ended_leaves() {
        let mut s = Session::new(0);
        s.on_event(Event::Admitted);
        assert_eq!(
            s.on_event(Event::CallEnded { at_ms: 9 }),
            Action::Leave(LeaveReason::CallEnded)
        );
    }

    #[test]
    fn time_alone_in_the_lobby_does_not_count_after_admission() {
        let mut s = Session::new(0);
        s.on_event(Event::Participants { at_ms: 0, count: 1 });
        s.on_event(Event::Admitted);
        assert_eq!(
            s.on_event(Event::Tick {
                at_ms: 3 * ALONE_TIMEOUT_MS
            }),
            Action::Continue
        );
        assert_eq!(
            s.on_event(Event::Participants {
                at_ms: 3 * ALONE_TIMEOUT_MS,
                count: 1
            }),
            Action::Continue
        );
    }

    #[test]
    fn the_notetakers_own_message_is_not_an_opt_out() {
        let mut s = admitted();
        let own = Event::Chat {
            text: "Notetaker here. Type opt out to stop recording.".into(),
            from_self: true,
        };
        assert_eq!(s.on_event(own), Action::Continue);
        assert_eq!(
            s.on_event(chat("opt out")),
            Action::Leave(LeaveReason::OptOut),
            "a member's message still counts after admission"
        );
    }

    #[test]
    fn empty_call_counts_as_alone_and_a_leave_decision_sticks() {
        let mut s = admitted();
        s.on_event(Event::Participants { at_ms: 0, count: 0 });
        assert_eq!(
            s.on_event(Event::Tick {
                at_ms: ALONE_TIMEOUT_MS
            }),
            Action::Leave(LeaveReason::Alone)
        );
        assert_eq!(
            s.on_event(Event::Participants {
                at_ms: ALONE_TIMEOUT_MS + 1,
                count: 4
            }),
            Action::Leave(LeaveReason::Alone)
        );
    }

    #[test]
    fn denied_leaves_before_admission_and_is_ignored_after() {
        let mut s = Session::new(0);
        assert_eq!(
            s.on_event(Event::Denied),
            Action::Leave(LeaveReason::Denied)
        );
        let mut s = admitted();
        assert_eq!(s.on_event(Event::Denied), Action::Continue);
    }

    #[test]
    fn removed_leaves_after_admission_and_is_ignored_before() {
        let mut s = Session::new(0);
        assert_eq!(s.on_event(Event::Removed), Action::Continue);
        let mut s = admitted();
        assert_eq!(
            s.on_event(Event::Removed),
            Action::Leave(LeaveReason::Removed)
        );
    }

    #[test]
    fn max_duration_leaves_at_the_cap_not_before() {
        let mut s = Session::new(1_000).with_max_call_ms(60_000);
        s.on_event(Event::Admitted);
        assert_eq!(s.on_event(Event::Tick { at_ms: 60_999 }), Action::Continue);
        assert_eq!(
            s.on_event(Event::Tick { at_ms: 61_000 }),
            Action::Leave(LeaveReason::MaxDuration)
        );
        // Default cap is 240 minutes.
        let mut s = admitted();
        assert_eq!(
            s.on_event(Event::Tick {
                at_ms: MAX_CALL_MS - 1
            }),
            Action::Continue
        );
        assert_eq!(
            s.on_event(Event::Tick { at_ms: MAX_CALL_MS }),
            Action::Leave(LeaveReason::MaxDuration)
        );
    }
}
