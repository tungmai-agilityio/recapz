# Browser recording

Status: **agreed with the maintainer (2026-11-19); built and merged (#296 to #298, #302), all tickets closed.**
Intent: #289

## Problem

The notetaker joins Google Meet by driving Chromium with one saved Google sign-in. Today that sign-in is a
personal account outside any Workspace: the account is the whole risk (a challenge or suspension stops every
join and touches a personal mailbox), Meet page changes break joins, Zoom has no join path yet, and every call
costs about 1 GB of server memory. A member who is already in the call has the audio in their own browser tab;
Recapz cannot use it except by uploading a file afterwards, which loses the live view and Live Ask.

## Decisions

- **Capture in the member's browser:** a member starts a recording from Recapz and shares the meeting tab with
  its audio (`getDisplayMedia`), and Recapz mixes it with their microphone (`getUserMedia`). A second mode,
  "In-room", records the microphone only, for meetings held in one room. No bot, no saved sign-in, no Chromium.
- **Stream, do not upload at the end:** the page sends 16 kHz mono 16-bit PCM in 100 ms frames over a WebSocket.
  The server writes the WAV as frames arrive, so closing the tab or a crash loses at most the frames in flight,
  and the live view and Live Ask work as they do for notetaker calls.
- **Reuse the call pipeline:** a `BrowserDriver` implements the existing `CallDriver` seam and runs under
  `run_meeting`, like the local replay driver. Recording, the live transcript, the transcript and report jobs,
  salvage after a restart and the usage cap stay one code path.
- **Mono mix:** the microphone and tab audio are mixed into one channel, matching today's pipeline. Speakers
  come from the speech provider's diarization, as for uploads. Separate channels for "me" and "others" is a
  later ticket.
- **Consent by the recorder:** with no bot there is no chat notice and no "opt out" phrase. Before every
  recording the member ticks "I have told everyone in this meeting that it is recorded and sent to
  <speech provider>", and the dialog offers that sentence to copy into the call chat. The recorder can stop and
  discard at any time (hard delete, as for the chat opt-out), and the owner can delete the meeting afterwards as
  today.
- **Reconnects:** the page buffers up to 60 s of frames while its socket is down and resends them after
  reconnecting; the server keeps the call open for 60 s without frames, then ends it and transcribes what it has
  (the existing `NoAudio` rule).
- **One recorder per call, owner only:** the socket is accepted only for the meeting's owner, only while it is
  recording, one socket at a time, and only from `base_url`'s origin.
- **Marked `platform = 'browser'`:** no migration. Browser calls do not count toward `max_concurrent_calls`,
  which protects Chromium memory; the monthly usage cap applies to their jobs as to any other call.
- **Keep the notetaker:** the Chromium notetaker stays, unchanged, for teams that sign it in. Browser recording
  is the default way to record on the Meetings page.
- **Supported browsers:** desktop Chrome and Edge, which share tab audio. Other browsers see a message that
  points to Upload. Zoom works through its web client in a tab; the Zoom desktop app is not a tab.

## Out of scope

- Separate microphone and tab channels; speaker names from the meeting page.
- System or desktop-app audio, a browser extension, a desktop app, mobile browsers, Safari and Firefox.
- Recording more than one tab, or two members recording the same call into one meeting.
- Automatic recording from the calendar: a member starts each recording.
- Removing the Chromium notetaker.

## Acceptance

- [ ] A member in desktop Chrome can start a recording of a Meet tab, sees the live transcript in the live view
      within a few seconds, stops, and gets a transcript and meeting report through the normal pipeline.
- [ ] "In-room" records the microphone only, with the same results.
- [ ] Recording cannot start until the consent box is ticked; the copied sentence names the configured speech
      provider.
- [ ] Sharing a tab without audio, or a window or screen, is refused with a message that says to share a tab
      with "Share tab audio" on.
- [ ] Stop and discard deletes the meeting, its audio and any live text, as the chat opt-out does.
- [ ] Losing the network for under 60 s loses no audio; closing the tab ends the call and what was recorded is
      transcribed.
- [ ] The socket refuses a member who is not the owner, a second socket, a meeting that is not recording, and a
      foreign origin.
- [ ] Browser calls never block or count toward notetaker calls under `max_concurrent_calls`.
- [ ] Audio goes only to the configured speech provider; logs hold meeting IDs, never titles, audio or text.
- [ ] `docs/spec.md` describes browser recording and its consent rule; `docs/ui-parity.md` has rows for it.

## Tickets

| # | Title (`type(scope): summary`) | Area | Blocked by | Touches | Size | Testing scope |
|---|---|---|---|---|---|---|
| 1 (#291) | feat(server): drive a call from browser audio | server, notetaker | none | `crates/server/src/browser_call.rs`, `crates/server/src/lib.rs`, `crates/server/src/notetaker_loop.rs` | small | `cargo test -p recapz-server` (touches meeting access: full workspace) |
| 2 (#293) | feat(server): accept browser recordings over a socket | server, auth | 1 | `crates/server/src/routes.rs`, `crates/server/src/routes/record.rs`, `crates/server/Cargo.toml`, `crates/server/tests/record.rs` | medium | `cargo test --workspace` (auth, meeting access) |
| 3 (#292) | feat(web): capture tab and microphone audio | web | none | `web/src/capture/` | medium | `pnpm -C web test`, lint |
| 4 (#294) | feat(web): record a meeting from this browser | web | 2, 3 | `web/src/pages/Meetings.tsx`, `web/src/pages/Record*.tsx`, `web/src/pages/Live.tsx`, `web/src/api.ts`, `docs/spec.md`, `docs/ui-parity.md` | medium | `pnpm -C web test`, lint, browser flow |

1. `BrowserDriver`: admitted at once, posts no chat notice, takes frames and stop requests from a per-meeting
   registry; "stop" ends the call, "discard" acts as the opt-out; reconnecting swaps the frame sender; 60 s
   without frames returns `NoAudio`. The cap query in `notetaker_loop` skips `platform = 'browser'`. Unit tests
   on the driver and the cap, no sockets.
2. `POST /api/recordings` (title, language, mode) creates the meeting owned by the caller with
   `platform = 'browser'` and starts `run_meeting` with the driver. `GET /api/meetings/{id}/record` upgrades to a
   WebSocket: binary frames of exactly 1600 samples (little-endian i16), text `stop` or `discard`; the refusals
   in Acceptance. Enables axum's `ws` feature (reason: the socket). Touches session checks and origin checks, so
   it is critical by `CLAUDE.md` and gets `needs-review`.
3. `startCapture(mode)` returns frames, input levels and state: tab audio plus microphone (echo cancellation
   on) or microphone only; an AudioWorklet mixes and resamples to 16 kHz frames of 1600 samples; the socket
   client buffers up to 60 s and reconnects with backoff; refuses a share without an audio track. Tests with fake
   media streams and a fake socket.
4. "Record from this browser" beside "Send notetaker" on Meetings; a dialog with title, language, mode, the
   consent box and the copy button; an unsupported-browser message linking to Upload. After start it opens the
   live view, which shows a recorder bar only in the recording tab: elapsed time, input levels, Stop, Stop and
   discard (confirm dialog), a warning when the shared tab's audio ends, and a leave-page warning. No prototype
   state exists for this flow: it is built from `web/src/ui/` primitives and the Upload screen's layout, and the
   ui-parity rows say so. Amends `docs/spec.md` (vocabulary, consent, data sent to providers).

## Open questions

- Does Chrome's echo cancellation remove the tab's audio from the microphone when the member uses speakers, or
  is the far side heard twice? Checked in ticket 3's live run; if not, the dialog recommends headphones. Answered
  by the implementer, blocks nothing.
- Does Meet keep sending tab audio while its tab is in the background? Checked in ticket 4's browser flow.
