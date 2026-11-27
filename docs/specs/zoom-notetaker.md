# Zoom notetaker

Status: **draft** (join path agreed with the maintainer 2026-11-27, ticket list awaiting review)
Intent: #693

## Problem

The notetaker joins Google Meet only. A Zoom link or calendar event is accepted and stored with `platform='zoom'`,
but nothing joins it: the scheduled claim has no platform filter, so the Meet driver picks the row up and fails
with `meet_ui_changed`. Zoom calls can be recorded today only from the macOS app or by uploading a recording.

## Decisions

- **Join path:** a Zoom Meeting SDK bot, as proposed in `docs/decisions/zoom-join-path.md`. Official route, and the
  SDK's raw data gives one audio stream per participant.
- **Speaker ribbon from per-participant audio:** no diarization for Zoom. Each participant stream is labelled with
  the participant name and mixed into the same 16 kHz mono WAV and `speakers.jsonl` the Meet path writes.
- **SDK runs in a separate sidecar process:** the SDK is native C++ for Linux and forces upgrades, so it must not
  link into the server binary. The sidecar joins, reads raw audio and chat, and talks to the server over a local
  WebSocket with the frame format `BrowserDriver` already uses (`crates/server/src/browser_call.rs`).
- **Reuse the call loop:** a `ZoomDriver` implements `CallDriver` (`crates/notetaker/src/driver.rs`). Session rules,
  opt-out detection, notice text, WAV output, live view and reports stay as they are.
- **Gate first:** until the driver ships, the claim skips non-Meet rows and marks Zoom meetings failed with a clear
  reason, instead of sending them to the Meet driver.
- **Notice and opt-out through Zoom chat:** the bot posts the recording notice in meeting chat and reads chat for
  opt-out phrases. If chat is disabled by the host, the existing "keep recording when the notice fails" option applies.
- **Own meetings first:** v1 targets meetings the team's Zoom account can authorise. External meetings depend on
  the app review outcome (see open questions).

## Out of scope

Teams, video capture, joining by Zoom phone dial-in, Zoom cloud recording import, the real-time media streams route,
changes to the macOS app (Zoom is already a known mic-holder app).

## Acceptance

- [ ] A queued Zoom meeting is never handed to the Meet driver.
- [ ] The bot joins a Zoom meeting from a link, waits in the waiting room, and leaves when the meeting ends,
      the bot is removed, or it is alone.
- [ ] The meeting has a transcript and a speaker ribbon with participant names, and a meeting report.
- [ ] The recording notice is posted in Zoom chat; an opt-out phrase in chat stops recording as in Meet.
- [ ] Members see "Zoom" as the platform label on the meetings list.
- [ ] The sidecar runs from the deploy bundle with its credentials read from the environment, never logged.

## Tickets

One ticket is one PR (about 300 source lines small, 600 medium). Order is by dependency.

| # | Title (`type(scope): summary`) | Area | Blocked by | Touches | Size | Testing scope |
|---|---|---|---|---|---|---|
| 1 | `fix(notetaker): do not hand Zoom meetings to the Meet driver` | server | none | `crates/core/src/notetaker.rs`, `crates/server/src/notetaker_loop.rs` | small | core + server tests |
| 2 | `docs(zoom): Zoom developer account and app review` (maintainer task) | tooling | none | `docs/decisions/zoom-join-path.md` | small | docs only |
| 3 | `feat(notetaker): sidecar protocol and ZoomDriver` | notetaker | 1 | `crates/notetaker/src/zoom/`, `crates/server/src/notetaker_loop.rs` | medium | notetaker + server tests with a fake sidecar |
| 4 | `feat(zoom-sidecar): join, waiting room and leave with the Meeting SDK` | tooling | 2, 3 | `zoom-sidecar/` | medium | sidecar build; live check on a real meeting |
| 5 | `feat(zoom-sidecar): per-participant audio and active speaker` | tooling | 4 | `zoom-sidecar/` | medium | sidecar tests; live check |
| 6 | `feat(zoom-sidecar): chat notice and opt-out chat events` | tooling | 4 | `zoom-sidecar/`, `crates/notetaker/src/zoom/` | small | sidecar + notetaker tests |
| 7 | `feat(deploy): run the Zoom sidecar next to the server` | tooling | 5 | `deploy/`, `docs/running-locally.md` | small | deploy script check |
| 8 | `feat(web): platform labels and Zoom copy` | web | none | `web/src/pages/Meetings.tsx`, `web/src/api.ts` | small | `pnpm -C web test`, UI parity rows |
| 9 | `docs: Zoom is supported, update spec and decision record` | tooling | 7 | `docs/spec.md`, `docs/specs/meet-notetaker.md`, `docs/decisions/` | small | docs only |

## Open questions

- Who owns the Zoom developer account, and can an unlisted app run for internal use? Blocks ticket 4. Maintainer.
- Must external meetings (hosted by other organisations) be joinable? If yes, the app review is required before
  they work and tickets 4 and 5 can only be verified on our own meetings until then. Maintainer.
- Does the SDK's raw-data permission need the host to approve recording on each meeting? Check against Zoom's
  current terms before ticket 4.
