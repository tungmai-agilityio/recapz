# Google Meet notetaker plan

Status: **maintainer decisions recorded (2026-11-12/13); plan approved. Rows 2 to 15 of the PR table in
section 10 are merged (#105 to #119); row 16, the live check with the maintainer, is the only open row.**
Facts marked *unverified* come from general knowledge and stay unverified until that live check settles
them.

## What exists today

- `crates/notetaker`: browser-free rules (`Session` with lobby, alone, call ended, opt-out, denied, removed
  and max-duration leave reasons, `WavWriter` at 16 kHz mono PCM, opt-out detection, speaker matching from
  active-speaker samples, strict join-link parsing), the `CallDriver` trait with a scripted fake, the
  Chromium launcher with process cleanup, in-page audio capture, and the Meet driver that joins, posts the
  notice, follows the call and leaves.
- `crates/core`: `purge_meeting`, calendar `sync` and `calendar_events`, encrypted Google refresh token
  storage, the jobs table with the `transcribe` job and the `label` job that names speakers from call
  samples.
- `crates/server`: Google sign-in, Google Calendar connect and disconnect, calendar sync with stored
  tokens, `POST /api/meetings/join`, the notetaker loop that books due events, claims scheduled meetings
  and starts calls at boot, and the `notetaker-login` and `notetaker-logout` commands.
- `web/`: upcoming meetings with an auto-join switch, a notetaker link box, and calendar connect on
  Recording & privacy.
- `deploy/`: Chromium, a virtual display and the secret key in the unit file and `docs/deploy.md`.
- `scripts/smoke-join.sh`: daily live check.

Missing: the live check (row 16). A live call has recorded audio (#130 to #132), but the Meet page labels and
every fact marked *unverified* still need the maintainer's run. The notetaker sign-in status has no
endpoint yet.

## Goals and non-goals

Goals for this plan: a notetaker that joins a Meet call from a pasted link or a calendar event, announces
itself, records the call audio to `audio/<meeting id>/`, leaves on the existing rules, and hands the
recording to the same `transcribe` job uploads use.

Non-goals: Zoom (comes after Meet; the join path in `docs/decisions/zoom-join-path.md` will be revisited then,
with Zoom's real-time media streams as a candidate that needs no bot), Teams, live view streaming, video capture,
hosted speech (the Deepgram speech provider, per `docs/decisions/hosted-speech.md`, unaffected by this plan).

## 1. Join path

### Options

| Option | How | Pros | Cons |
|---|---|---|---|
| **A. Chromium on the Meet web client** (chosen) | Rust drives Chromium over the DevTools protocol | Works on any Meet call the account can reach; no Google review; audio stays on our server | Page changes break it; automation may be detected; terms risk (see Risks) |
| B. Meet Media API | Official real-time media over WebRTC | Supported interface, no participant in the call | Never left Developer Preview; Google announced its shutdown on 2026-10-28, endpoints off on 2027-01-15 ([release notes](https://developers.google.com/workspace/meet/release-notes)) |
| C. Meet REST API after the call | Read Google's own recording or transcript | No bot at all | Needs Workspace plans with recording/transcripts; no Vietnamese quality data; no opt-out in call |
| D. Scrape in-call captions | Read caption text from the page | No audio handling | Caption languages and quality are Google's, not ours; still a bot |

Decision (maintainer, 2026-11-12): **A for v1.** B is being shut down, so it is not an option. Upload stays
the fallback for calls the notetaker cannot join.

### Identity: signed-in account vs guest

Decision (maintainer, 2026-11-12): **a dedicated Workspace account in the team's domain.**

| | Dedicated Workspace account in the team's domain (chosen) | Free Google account | Guest (not signed in) |
|---|---|---|---|
| Admit | Same-domain calls usually admit without knocking *(unverified, depends on the call's access setting)* | Knocks on every call | Knocks; many Workspace domains block anonymous guests |
| Calendar invites | Can be invited like a person | Same | Not possible |
| Detection risk | Lowest: real account, persistent profile | Medium | Highest |
| Cost | One Workspace seat | Free | Free |

The account, such as `notetaker@<team domain>`, is signed in **once by a human** through a
`recapz notetaker-login` command that opens a visible Chromium window on the server's display (or over
SSH X forwarding / VNC). The browser profile is kept in `data/notetaker-profile/` (mode 0700). Automated
password entry is not built: it trips Google's sign-in protection and needs 2FA handling.

The profile directory is a credential. It is excluded from backups (decided 2026-11-13; re-login is
cheap), never logged, and removed by `recapz notetaker-logout`.

### Join flow

1. A meeting reaches its start time (see Scheduling) and moves `scheduled → joining`.
2. Launch one Chromium per meeting with the shared profile copied to a temp dir (two calls at once must
   not share a live profile; *unverified* whether copying keeps the session valid; fallback is one call
   at a time).
3. Camera and microphone are off before joining. Chromium runs with fake media devices so no real device
   is ever opened, and the notetaker never sends audio.
4. Open the link, set the display name when the page asks (guest case only), click join.
5. Lobby: feed `Session` regular ticks. Outcomes: admitted, denied (new `LeaveReason::Denied`), or the
   existing 10-minute lobby timeout.
6. On admission: `joining → recording`, post the join notice in chat, start audio capture and the
   250 ms participant-count and active-speaker sampling that `Session` and `speakers.rs` already expect.
7. Leave on any `Action::Leave`, click the leave control, close the browser, kill its process group.

Page selectors live in one file (`crates/notetaker/src/meet/selectors.js` or similar), prefer ARIA roles
and labels over class names, and every lookup that fails reports a named step (`join_button`,
`chat_input`, ...) so a page change shows up as `meet_ui_changed: <step>` instead of a hang.

### Consent and recording notice

- Display name: the configured notetaker name (Recording & privacy), default "Recapz notetaker".
- On admission it posts the configured join message in call chat, for example: "Recapz notetaker is
  recording and transcribing this call for {owner name}. Type 'opt out' to make it leave and delete the
  recording." It is posted once per call.
- If the chat message cannot be posted (chat disabled for participants), the notetaker **leaves**:
  without the notice there is no consent and no way to opt out. Decision (maintainer, 2026-11-13).
- Opt-out works from the lobby onward (already in `Session`) and calls `purge_meeting`.
- If a hosted speech provider is enabled, the join message also names it (open question in
  `docs/decisions/hosted-speech.md`, proposed yes).
- Meet's own recording indicator does not appear for this kind of capture *(unverified)*; the chat
  notice and the visible participant name are the disclosure. Local recording-consent law (including
  Vietnam's personal data decree) is the maintainer's call; this plan is not legal advice.

## 2. Audio capture

### Options

| | In-page capture (chosen) | OS audio sink |
|---|---|---|
| How | Script hooks remote WebRTC audio tracks, mixes them in an `AudioContext` at 16 kHz, an `AudioWorklet` posts 16-bit frames to Rust through a DevTools binding | Chromium plays to a PulseAudio null sink per call; Rust reads the monitor source |
| Deps | None beyond Chromium | PulseAudio on the server, one sink per call; BlackHole or similar on macOS for dev |
| Breaks when | Meet changes how it attaches audio *(unverified how stable)* | Rarely |
| Dev on the Mac | Same code | Different code path |

Decision: **in-page capture for v1**, OS sink as the fallback if the hook proves fragile.
Meet sends a few mixed "loudest speaker" audio streams rather than one per person *(unverified)*, so
the capture is a mix and speaker names come from active-speaker samples, as `speakers.rs` already
assumes.

### Feeding the pipeline

1. Frames arrive about every 100 ms as base64 over the binding; the driver decodes them and writes to
   `WavWriter` at `audio/<meeting id>/recording.wav` (the folder `purge_meeting` already deletes), and
   sets `meetings.audio_path`.
2. Active-speaker samples go to `audio/<meeting id>/speakers.jsonl` (no new table; purge already covers
   the folder). The display names in that file are participant names, so it is never logged.
3. On leave (except opt-out): `WavWriter::finish`, `recording → transcribing`, queue the same
   `transcribe` job uploads use. A new `label` job runs after it and fills `speakers` and
   `segments.speaker_id` from `speakers.jsonl` with `speaker_for`.
4. Opt-out: stop capture, `purge_meeting`, status `opted_out`.
5. Audio goes only to the configured speech provider. The join message used to name it; that rule was dropped on
   2026-11-20 (#323 to #326) and the spec section it cited no longer exists.
6. Guard: if no audio frame arrives for 60 s while admitted, log a warning with the meeting id; after
   5 minutes, leave with `DriverError(no_audio)` and transcribe what was captured.

## 3. Scheduling and the driver

- A notetaker loop in the server (not a job handler: a call lasts hours and job locks are short) wakes
  every 15 s and claims `scheduled` meetings whose calendar start is within 1 minute, or that were
  pasted (no start time), by moving them to `joining` in one `UPDATE ... WHERE status = 'scheduled'`.
- `max_concurrent_calls` in config, default 2 (one Chromium uses a few hundred MB *(unverified)*; the
  server also waits on the hosted speech provider).
- `CallDriver` trait in `crates/notetaker` with a Meet implementation and a scripted fake, so the loop is
  tested without a browser.
- Restart recovery: on start, any meeting left in `joining` fails with `server_restarted`; any meeting in
  `recording` gets its WAV header repaired (the samples are on disk), moves to `transcribing` and is
  marked with a note that the recording may be cut short. No automatic rejoin in v1.
- Hard cap: leave after `max_call_minutes` (default 240, counted from the join attempt) with `LeaveReason::MaxDuration`.
- Spoken language: a calendar meeting is created with `vi`. The owner or an admin can set `vi`, `en` or
  `multi` with `PATCH /api/meetings/{id}/language` while the meeting is `scheduled` (409 after the loop
  claims it as `joining`). The live stream and the batch transcript both read the column once the call is
  admitted, so the choice applies to both. There is no tenant-wide default yet.
- Cross-member dedupe (two members booking the same call) stays an open issue; the loop joins once per
  meeting row.

## 4. Calendar integration and scopes

- Sign-in keeps `openid email profile`. Calendar is a separate **Connect calendar** action on
  Recording & privacy that requests `https://www.googleapis.com/auth/calendar.events.readonly` with
  `access_type=offline`, `prompt=consent` and `include_granted_scopes=true` (incremental consent).
- The callback checks the granted `scope` list; a member can untick calendar on the consent screen, and
  then nothing is stored.
- Sync: every 10 minutes per connected member, `events.list` on the primary calendar for the next
  7 days, through the existing `parse_page` caps, into `calendar::sync`. `auto_join` events book a
  meeting as they do today.
- **Google verification** *(unverified, check the current policy)*: calendar read scopes are classed
  as sensitive. For an app whose consent screen is **Internal** (all members in one Workspace
  organization) no verification is needed. For **External**, an app in Testing mode is limited to listed
  test users and its refresh tokens expire after 7 days; publishing needs brand and sensitive-scope
  verification (no third-party security assessment, which is for restricted scopes). See Google's
  [production readiness](https://developers.google.com/identity/protocols/oauth2/production-readiness/overview)
  and [unverified apps](https://support.google.com/cloud/answer/7454865) pages.
- Decision (maintainer, 2026-11-13): **Internal.** Only members in the team's Workspace domain can
  connect a calendar; no Google verification is needed. A member outside the domain gets a clear error.

## 5. Token storage

- New additive tenant migration: `google_tokens(member_id PRIMARY KEY REFERENCES members(id) ON DELETE
  CASCADE, refresh_token BLOB NOT NULL, nonce BLOB NOT NULL, scopes TEXT NOT NULL, created_at INTEGER,
  updated_at INTEGER)`.
- Refresh tokens are encrypted with XChaCha20-Poly1305 (new dependency `chacha20poly1305`; reason:
  authenticated encryption of refresh tokens at rest). The 32-byte key comes from `RECAPZ_SECRET_KEY`
  in the systemd environment file, never `config.toml` or the data dir, so a copy of the backups alone
  cannot use the tokens. The server refuses to start calendar sync without a key.
- Access tokens live in memory only, refreshed when under 5 minutes from expiry.
- `invalid_grant` on refresh (revoked, password change, Testing-mode expiry): delete the row, mark the
  member's calendar as disconnected, show "Reconnect calendar". Events already synced stay.
- Disconnect and member removal: call Google's revoke endpoint (best effort) and hard-delete the row.
- Tokens and keys are never logged; errors carry the member id only.

## 6. Failure and leave handling

| Case | Detected by | Result |
|---|---|---|
| Lobby, nobody admits | `Session` lobby timeout (10 min) | `failed`, "Nobody admitted the notetaker" |
| Join denied | Denied page state | `failed`, new `LeaveReason::Denied` |
| Removed by a participant | Removed page state while admitted | Keep audio, transcribe; new `LeaveReason::Removed` |
| Call ended | `CallEnded` | Transcribe |
| Alone 30 s | `Session` | Transcribe |
| Opt-out in chat | `Session` | `purge_meeting`, `opted_out` |
| Chat notice not posted | Driver | Leave before capture starts, `failed`, "Chat is off, so the notetaker could not announce itself" |
| Page not recognised | Selector step fails | `failed`, `meet_ui_changed: <step>`; no screenshot (it would hold faces and names) |
| Account signed out | Sign-in page instead of the call | `failed`, `notetaker_signed_out`; admins see it on Recording & privacy |
| Browser crash or DevTools disconnect | Driver | Finish WAV, transcribe if over 30 s of audio, else `failed` |
| No audio frames | 5 min guard | As browser crash |
| Too long | `max_call_minutes` | `MaxDuration`, transcribe |
| Server restart | Startup recovery | See section 3 |

Always: the browser process group is killed in a drop guard, and the temp profile copy is deleted.

## 7. Test strategy without real meetings

1. **Rules, no browser.** Extend `Session` tests for `Denied`, `Removed`, `MaxDuration`, notice failure.
2. **Loop with a fake driver.** Scripted `CallDriver` events drive the server loop against a temp tenant:
   status transitions, WAV at the right path, `transcribe` and `label` jobs queued, `purge_meeting` on
   opt-out, restart recovery, concurrency cap. Exact values, counts asserted before loops.
3. **Capture against a local fixture page.** A small page of our own (served by the test) plays a
   generated 440 Hz tone through an in-page WebRTC loopback (two peer connections). Real Chromium runs
   the capture script; the test asserts the WAV length (±200 ms) and the dominant frequency. Behind a
   `browser-tests` feature and `make test-browser`, skipped when no Chromium is found (it prints that it
   skipped; it never passes silently).
4. **Join flow against a fixture page.** The same fixture exposes the roles and labels the selectors
   use and can switch between lobby, admitted, denied, removed, ended and chat-disabled states, so every
   row of section 6 that comes from the page is covered. The fixture is our own minimal markup, not a
   copy of the Meet page.
5. **Calendar and tokens.** Mock Google token and calendar endpoints (`base_url` override): incremental
   consent with the scope unticked, refresh, `invalid_grant`, revoke on disconnect, encrypt/decrypt
   round trip, wrong key fails.
6. **Live, manual, once per milestone.** The maintainer opens a test call; the checklist covers admit,
   denied, opt-out, chat off, removal, a call hosted outside the team's domain, 30-minute recording,
   Vietnamese and English speech. Then `scripts/smoke-join.sh` runs daily on the server with `MEET_TEST_URL`.

The fixture tests prove our code works against our model of the page. Only the live check proves the
model matches Meet; that gap is accepted and called out in each PR.

## 8. Risks

| Risk | Likelihood | Effect | Mitigation |
|---|---|---|---|
| Meet page changes | High, ongoing | Joins fail | One selectors file, ARIA first, named failure steps, daily smoke test, upload fallback |
| Automation detected, account challenged or suspended | Medium *(unverified)* | No joins until re-login | Visible (non-headless) Chromium under a virtual display on the server, persistent profile, real account, low volume, no scripted sign-in |
| Google terms on automated access | Real *(not legal advice)* | Account suspension | Internal use only, dedicated account; no official replacement exists once the Media API shuts down |
| Workspace settings block the account | Medium | Knocks or denied | Same-domain account; `Denied` reported clearly |
| Mixed audio only | Likely | Weaker speaker labels | Active-speaker samples; diarization later |
| Chromium plus job processing on one small server | Medium | Memory pressure | `max_concurrent_calls`, transcription runs after the call |
| Refresh tokens leak via backups | Low | Calendar read access | Encrypted, key outside the data dir |
| Recording without valid consent | Low with the notice | Legal | Notice or leave, opt-out, Recording & privacy settings |

## 9. Deploy

- The server needs Chromium and a virtual display (Xvfb) for visible mode; the systemd unit gains both
  and `RECAPZ_SECRET_KEY` in its environment file. `deploy/` and `docs/deploy.md` change in one PR.
- Dev on the Mac uses the installed Chrome or Chromium, visible window, same code.
- Chromium path is `notetaker.chromium_path` in config; empty means search the usual locations.

## 10. PR breakdown

Each row is one concern of about 300 source lines or fewer, in dependency order. Rows 2-4 and 5-7 are
two independent tracks.

| # | PR | Depends on | Done when |
|---|---|---|---|
| 1 | `docs: plan the Meet notetaker` (#103) | — | Approved by the maintainer |
| 2 | `feat(notetaker): add denied, removed and max-duration leave reasons` (#105) | 1 | `Session` tests for each new reason |
| 3 | `feat(notetaker): add the call driver trait and a scripted fake` (#106) | 2 | Fake replays events; trait carries audio frames and speaker samples |
| 4 | `feat(server): run the notetaker loop for scheduled meetings` (#107) | 3 | Fake-driver tests from section 7.2, including restart recovery and the concurrency cap. Full workspace tests (touches meeting status) |
| 5 | `feat(core): store encrypted Google refresh tokens` (#108) | 1 | Migration, round trip, wrong key fails, hard delete. Full workspace tests |
| 6 | `feat(server): connect and disconnect Google Calendar` (#109) | 5 | Mock OAuth tests: scope granted, scope unticked, revoke on disconnect |
| 7 | `feat(server): sync calendars with stored tokens` (#110) | 6 | Mock calendar tests: sync, refresh, `invalid_grant` disconnects |
| 8 | `feat(notetaker): launch Chromium and clean up its processes` (#111) | 3 | Launch, connect, kill on drop; profile copy deleted |
| 9 | `feat(notetaker): capture call audio in the page` (#112) | 8 | Tone fixture test (section 7.3) |
| 10 | `feat(notetaker): join a Meet call and post the notice` (#113) | 9 | Fixture states for lobby, admit, denied, chat off |
| 11 | `feat(notetaker): follow the call and leave` (#114) | 10 | Fixture states for removed, ended, participant count, chat opt-out, active speaker |
| 12 | `feat(server): add notetaker login and logout commands` (#115) | 8 | Profile created with 0700; logout removes it |
| 13 | `feat(core): label speakers from call samples` (#116) | 4 | `label` job assigns names from `speakers.jsonl`; retries do not duplicate |
| 14 | `feat(web): connect calendar and notetaker status on Recording & privacy` (#118) | 6, 12 | Matches the prototype per `docs/ui-parity.md`, both skins, mobile |
| 15 | `chore(deploy): add Chromium, a virtual display and the secret key` (#117) | 12 | Unit file and `docs/deploy.md` updated |
| 16 | Live check with the maintainer, no PR | 4, 7, 11, 13, 15 | Section 7.6 checklist passes; notes added under Implementation notes below |

New dependencies, each with its reason in its PR: a DevTools protocol client (`chromiumoxide`,
*unverified that it is still maintained; fallback is a small hand-written client over
`tokio-tungstenite`*) in PR 8, `chacha20poly1305` in PR 5.

## Maintainer decisions

Recorded 2026-11-12 and 2026-11-13.

1. Meet first with the browser notetaker; Zoom later, its join path revisited then.
2. Notetaker identity: a dedicated Workspace account in the team's domain.
3. OAuth consent screen: Internal.
4. When the chat notice cannot be posted, the notetaker leaves.
5. External calls are in scope: the notetaker knocks on other companies' calls, and the live check
   includes one.
6. The Chromium profile is excluded from backups.

## Implementation notes

Assumptions made while building, moved from `docs/review-notes.md`. Items marked unverified, and every page
label, must be confirmed in the live check (row 16).

### Meet notetaker loop (assumptions)
- The notetaker claims every `scheduled` meeting that has a join link, oldest first within a workspace and one call per workspace in turn (the workspace that has gone longest without a call first), up to `max_concurrent_calls` minus the calls already `joining` or `recording`. Calendar booking already creates the row about when the call starts, so the loop does not re-check the start time. Revisit if booking moves earlier than one minute.
- Opt-out calls `purge_meeting`, which deletes the meeting row, so no `opted_out` status remains to show. Confirm that is the intended outcome; keeping a stub row would need a purge variant.
- The "may be cut short" note set on a recovered recording is cleared by the transcribe job when it succeeds (it resets `error`). Say so if the note should survive.

### Calendar sync (assumptions)
- Sync runs every 10 minutes per connected member for the next 7 days, one member at a time, and starts only when `google.client_id` and `RECAPZ_SECRET_KEY` are set. A malformed key stops the server at start.
- `invalid_grant` deletes the token and sets a "reconnect" marker in the tenant `settings` table (`google_reconnect:<member>`); `GET /api/calendar` returns `needs_reconnect` for it, and saving a new token or disconnecting clears it. Events already synced stay.

### Notetaker browser launch (assumptions)
- Uses `chromiumoxide` 0.9.1 (released and building on Rust 1.85+, so it looks maintained; the hand-written fallback in the plan was not needed). Chromium is started by `std::process::Command` in its own process group, then `chromiumoxide` only connects to its debug port, so cleanup does not depend on the library.
- The process group is killed through `kill(1)` instead of a `libc` call, to avoid one more dependency. Unix only (macOS for dev, Linux for the server).
- The per-call work folder is created with mode 0700 and deleted with the browser. Callers should put it under the data folder (for example `data/notetaker-tmp/<meeting id>`). Backups do not exclude it yet; the deploy PR will exclude `notetaker-profile/` and `notetaker-tmp/`.
- Chromium listens for DevTools on a random localhost port, so any local user on the server can reach a running notetaker browser. Keep the server single-purpose or restrict local logins.
- A server killed without a handler (SIGKILL) leaves its browser and work folder behind; the loop PR should sweep `notetaker-tmp/` at startup.
- `make test-browser` runs the real-browser test; with no Chromium it prints SKIPPED and returns (it does not fail the build).

### Notetaker audio capture (assumptions)
- Capture hooks `RTCPeerConnection` in the page before its own scripts run, so a call page that builds its connections through another constructor (or in a worker) would be missed. The tone test proves the mechanism against our own loopback page, not against the real call page; only the live check can prove that.
- The mix is one stream: if several remote tracks carry the same speaker they add up and the result is clamped. A very loud overlap can clip.
- The capture needs a secure context (https or localhost) and the `--autoplay-policy=no-user-gesture-required` flag, which the launcher now sets.
- Any script on the call page can call the audio binding and so inject fake audio; that is the same trust level as the page's own audio. The server drops payloads over 8 KB unread and drops frames when its queue is full, so a hostile page cannot grow server memory.
- The capture worklet loads from a `blob:` URL; a call page with a strict content security policy for workers could block it (unverified for Meet, checked in the live check). Only the standard `RTCPeerConnection` name is hooked, not `webkitRTCPeerConnection`, and only the top frame is captured.

### Notetaker join flow (assumptions)
- Every label the notetaker looks for on the call page (join and ask-to-join buttons, camera and microphone toggles, leave button, chat button, message box, send button, lobby and denied texts, sign-in link) is in one table in `crates/notetaker/src/meet/page.js` and is our guess at the page. The tests run against our own fixture, so they prove the flow, not the labels. The live check must confirm each one; a wrong one reports `meet_ui_changed: <step>`.
- Typing the notice uses a real text-insert event (not a script that sets the value), then checks the message shows in the chat log before it counts as posted; no message in 3 seconds is `meet_ui_changed: chat_send`.
- Chat disabled (button disabled or an off-notice on the page) is `ChatOff`: the notetaker must leave and never start capture. The caller (the follow step) is responsible for leaving.
- Guest sign-in (typing a display name) is not built: the notetaker uses a signed-in account. A signed-out page is reported as `SignedOut`.
- Camera and microphone toggles are only clicked when the page offers "turn off"; the fake devices in the launcher mean a real device is never opened either way.
- Capture installs its hook before the page loads, so audio frames can flow while the notetaker is still in the lobby. The follow step must write no frames to disk until `join` returns `Admitted` (notice posted), and must leave the call when `join` returns an error.
- A join click that has no effect (the join page still showing after `unknown_limit`) is reported as `meet_ui_changed: join_button`, and the notice only counts as posted when the number of chat messages starting like it goes up.

### Notetaker following the call (assumptions)
- Participant count, who is speaking, the chat log, "removed", "ended" and the rejoin button are looked up in `meet/page.js` by labels we modelled ("N participants", "Name, speaking", "You've been removed from the call", "Rejoin"). Like the join labels they are unverified against Meet; the live check must confirm each. Chat texts from others arrive as the page shows them, sender name included, so the opt-out words match whole words anywhere in the line.
- `MeetDriver` looks at the page every 250 ms and drops audio frames until admission and the posted notice; it returns `NoAudio` after 5 minutes without a frame (the plan's 60-second warning log is not added: this crate does not log).
- Speaker sample times count from admission, while the recording starts at the first frame after it; the gap is a few hundred milliseconds, inside the matching window used by `speaker_for`.
- Browser tests now take a shared lock so only one Chromium runs at a time; with several at once the audio timing test missed its 200 ms tolerance on a busy machine.
- Our own chat lines are recognised by a leading "You:"; a participant named "You Li" is not mistaken for the notetaker. If the real page labels our own lines differently, opt-outs from others would still be seen (they are never flagged as ours) but our notice could be re-read as a member message; the live check covers this.
- The chat line count is clamped when the log shrinks (panel closed and reopened), so an opt-out typed after a reopen is still seen. Lines re-rendered after a reopen can be reported again, which is harmless once a leave decision exists.

### Notetaker login (assumptions)
- `recapz notetaker-login` runs Chromium directly on `data/notetaker-profile/` (mode 0700, tightened if it already existed) with no debug port, so only the person at the window types the password. It succeeds only if the browser exits cleanly and wrote a profile. On Linux it needs `DISPLAY` or `WAYLAND_DISPLAY`; the error tells the person to use SSH X forwarding or VNC. A signed-out session later shows up as `notetaker_signed_out` on the meeting.
- The login window is a separate Chromium instance from calls; calls copy the profile, so a login or logout never disturbs a call in progress, but a call already running keeps its copy until it ends.
- `logout` hard deletes the profile folder and is safe to repeat. Excluding `notetaker-profile/` from backups is applied in the deploy PR.
- `login` only proves the browser exited cleanly and wrote a profile, not that the person finished signing in (Chromium creates the profile on first start), so it says "profile saved" and asks for a test call. `logout` does not check for an open login window; delete it only when no login window is open.

### Labelling speakers (PR 13)
- `speakers.rs` moved from the notetaker crate to core (the notetaker re-exports it), so the `label` job can use `speaker_for` without core depending on a browser crate.
- Sample times count from admission, the same start as the recording, so they line up with segment times. Not checked against a real call.
- The `transcribe` job queues `label` first, then `report` and `index`, only when the call's `speakers.jsonl` exists. Jobs run oldest first, so the report reads named speakers. If `label` itself fails and is retried, the report may already have run with generic names; accepted for v1.
- Each name becomes a `speakers` row with `label` and `display_name` set to the call's display name; members are not matched. Segments with no samples stay unnamed.

### Deploy for the notetaker (PR 15)
- Xvfb runs as its own unit on `:99` and the server unit sets `DISPLAY=:99`; not run on a real host. Chromium's package name and its user-namespace policy differ by distribution, so the docs say what is needed instead of one command for all.
- `/etc/recapz/recapz.env` must exist (the unit reads it); the deploy steps create it empty with mode 0640.
- The backup script skips `notetaker-profile/` and `notetaker-tmp/`. Checked with a throwaway data folder: the recording and the database copies are kept, both cookie files are not.

### Calendar connect on Recording & privacy (PR 14)
- The prototype's Recording & privacy screen needs a settings API that does not exist yet, so this PR adds the screen with only the calendar row; the other rows stay unticked in `docs/ui-parity.md`. The tile copies the prototype's Connections tile (logo, name, one line, badge) inside the screen's boxed row layout.
- The Google callback now redirects to `/recording?calendar=<outcome>` (it was `/`), so the member lands where the outcome is shown.
- Notetaker sign-in status is not shown: the server has no endpoint for it. Add one with the settings API, then a status row here.
- Checked by eye in the in-app browser: Daylight and Ink, mobile width. Dark mode was not checked beyond the shared tokens. Screenshot of this screen and the prototype's are in the thread.
- At the time of this PR the server did not serve the built web app, so outside `make dev` the redirect to `/recording?...` had no page to land on. Serving the app has since merged (#281), so the redirect now lands on the page.

### Starting the notetaker loop (follow-up to PR 4)
- PR 4 built the claim, run and recovery rules but nothing started them. `notetaker_loop` now does, every 15 s: book calendar events that start within a minute (`book_due`), claim scheduled meetings up to `max_concurrent_calls` (counted across tenants), and start each call. Startup runs `recover` for every tenant and deletes `notetaker-tmp/` (left by a killed server) before the loop begins. The loop is off, with a warning, when no Chromium is found.
- The cap is checked across tenants: active calls (`joining`, `recording`) of every tenant are counted before any claim. A tenant that errors during a pass is logged and skipped. A call that ends in an error or panics is settled by `salvage`: a recording with audio keeps it (WAV header repaired, then transcribed, with a note that it may be cut short), anything else fails with `notetaker_error`; only that meeting is touched.
- Each call runs on its own thread with its own current-thread runtime, because `run_meeting` holds a database handle across waits and so cannot be moved between threads. A stuck call cannot stall the server's runtime either. The thread count is bounded by `max_concurrent_calls`.
- A call that cannot start fails the meeting with `notetaker_signed_out` (no saved profile) or `notetaker_no_display` (Linux without `DISPLAY`) and never launches a browser. Any error out of the call itself becomes `notetaker_error` (details in the log, by meeting id only).
- The chat notice is a constant for now ("Recapz notetaker is recording and transcribing this call. Type 'opt out' ..."): the notetaker name and join message settings need the settings API. The name shown in the participant list is whatever the saved Google account is called.
- `max_call_minutes` is converted to ms here; 0 is treated as 1 minute.
- Not tested: a real call through the loop (needs the live check, PR 16); the thread start and the Meet driver wiring are only compiled.

### Meet page debug dump (after the first live run)
- The first live run failed with `meet_ui_changed: join_button` and the failure left no trace of what the page showed. `RECAPZ_NOTETAKER_DEBUG=1` now saves a screenshot and the page's buttons when a page step fails (private folder, off by default, excluded from backups). Tested on our own fixture page only.
### Audio capture on the live page (after "No audio was captured")
- Theory tested and dropped: Meet's page policy blocking the blob worklet. A page with a Meet-shaped policy (nonce, `strict-dynamic`, trusted types, `eval` proven refused) still loaded the worklet and captured the tone, and Meet's own second policy header allows `blob:`. So the cause of the live run's zero frames is still unknown: either our hook never saw a Meet audio track, or the call ended early for another reason.
- Added so the next run says which: at the end of a call the server logs `audio capture: peers=<n> tracks=<n> frames=<n> path=<worklet|processor: why>` plus how many frames were dropped before admission. Counts and one word only.
- Added insurance: if the worklet is refused or fails, a script processor delivers the same audio (tested by making `addModule` reject). The hook also covers the `webkitRTCPeerConnection` alias, not tested against a page that uses it.
- `notetaker` now depends on `tracing` (already used by the server) to log the capture line.
- Cause found: on the live call page (signed in, `crossOriginIsolated` true) `URL.createObjectURL` returns `blob:null/…`, and `addModule` fails with `AbortError` ("a dependency or cross-origin script failed to load"). The worklet now loads from a `data:` URL; on the same page that gave `path=worklet` and frames, and a live call recorded audio. Local fixtures with COOP/COEP or Document-Isolation-Policy still mint `blob:http://…` URLs, so there is no automated test for the opaque origin; the processor fallback stays as insurance.
