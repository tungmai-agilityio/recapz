# Google Meet notetaker plan

Status: **proposed, needs the maintainer's approval.** Nothing in this plan is built. Facts marked
*unverified* come from general knowledge and must be checked against Google's current docs before the
PR that depends on them. Decisions marked **(pending)** wait on the maintainer.

## What exists today

- `crates/notetaker`: browser-free rules. `Session` (lobby 10 min, alone 2 min, call ended, opt-out),
  `WavWriter` (16 kHz mono PCM), opt-out detection, speaker matching from active-speaker samples,
  strict join-link parsing.
- `crates/core`: `purge_meeting` (deletes `audio/<meeting id>/` then rows), calendar `sync` and the
  `calendar_events` table, the jobs table and the `transcribe` job used by uploads.
- `crates/server`: Google sign-in (`openid email profile`, invite-only, `{base_url}/auth/callback`),
  `POST /api/meetings/join` that inserts a `scheduled` meeting, calendar page parsing with size caps
  (no HTTP fetch, no calendar scope, no token storage).
- `scripts/smoke-join.sh`: daily live check, never run.

Missing: anything that starts a browser, joins a call, captures audio, stores Google tokens or fetches
calendars. A `scheduled` meeting stays `scheduled` forever.

## Goals and non-goals

Goals for this plan: a notetaker that joins a Meet call from a pasted link or a calendar event, announces
itself, records the call audio to `audio/<meeting id>/`, leaves on the existing rules, and hands the
recording to the same `transcribe` job uploads use.

Non-goals: Zoom (Meeting SDK bot, `docs/zoom-join-path.md`), Teams, live view streaming, video capture,
hosted speech (opt-in per `docs/hosted-speech.md`, unaffected by this plan).

## 1. Join path

### Options

| Option | How | Pros | Cons |
|---|---|---|---|
| **A. Chromium on the Meet web client** (proposed) | Rust drives Chromium over the DevTools protocol | Works on any Meet call the account can reach; no Google review; audio stays on our server | Page changes break it; automation may be detected; terms risk (see Risks) |
| B. Meet Media API | Official real-time media over WebRTC | Supported interface, per-participant audio | Developer Preview *(unverified as of 2026-11)*; enrollment and restricted scopes; likely limited to calls in our own Workspace |
| C. Meet REST API after the call | Read Google's own recording or transcript | No bot at all | Needs Workspace plans with recording/transcripts; no Vietnamese quality data; no opt-out in call |
| D. Scrape in-call captions | Read caption text from the page | No audio handling | Caption languages and quality are Google's, not our whisper; still a bot |

Decision: **A for v1**, keep B as the replacement to revisit when it leaves preview. Upload stays the
fallback for calls the notetaker cannot join.

### Identity: signed-in account vs guest **(pending)**

| | Dedicated Workspace account in the team's domain (proposed) | Free Google account | Guest (not signed in) |
|---|---|---|---|
| Admit | Same-domain calls usually admit without knocking *(unverified, depends on the call's access setting)* | Knocks on every call | Knocks; many Workspace domains block anonymous guests |
| Calendar invites | Can be invited like a person | Same | Not possible |
| Detection risk | Lowest: real account, persistent profile | Medium | Highest |
| Cost | One Workspace seat | Free | Free |

Proposed: a dedicated account such as `notetaker@<team domain>`, signed in **once by a human** through a
`recapz notetaker-login` command that opens a visible Chromium window on the server's display (or over
SSH X forwarding / VNC). The browser profile is kept in `data/notetaker-profile/` (mode 0700). Automated
password entry is not built: it trips Google's sign-in protection and needs 2FA handling.

The profile directory is a credential. It is excluded from backups (re-login is cheap), never logged, and
removed by `recapz notetaker-logout`.

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
  without the notice there is no consent and no way to opt out. Proposed default; the maintainer can
  relax it.
- Opt-out works from the lobby onward (already in `Session`) and calls `purge_meeting`.
- If a hosted speech provider is enabled, the join message also names it (open question in
  `docs/hosted-speech.md`, proposed yes).
- Meet's own recording indicator does not appear for this kind of capture *(unverified)*; the chat
  notice and the visible participant name are the disclosure. Local recording-consent law (including
  Vietnam's personal data decree) is the maintainer's call; this plan is not legal advice.

## 2. Audio capture

### Options

| | In-page capture (proposed) | OS audio sink |
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
5. Audio never leaves the server. Transcription is local whisper unless the deployment opted in to hosted
   speech; this plan changes neither rule.
6. Guard: if no audio frame arrives for 60 s while admitted, log a warning with the meeting id; after
   5 minutes, leave with `DriverError(no_audio)` and transcribe what was captured.

## 3. Scheduling and the driver

- A notetaker loop in the server (not a job handler: a call lasts hours and job locks are short) wakes
  every 15 s and claims `scheduled` meetings whose calendar start is within 1 minute, or that were
  pasted (no start time), by moving them to `joining` in one `UPDATE ... WHERE status = 'scheduled'`.
- `max_concurrent_calls` in config, default 2 (one Chromium uses a few hundred MB *(unverified)*; the
  server also runs whisper).
- `CallDriver` trait in `crates/notetaker` with a Meet implementation and a scripted fake, so the loop is
  tested without a browser.
- Restart recovery: on start, any meeting left in `joining` fails with `server_restarted`; any meeting in
  `recording` gets its WAV header repaired (the samples are on disk), moves to `transcribing` and is
  marked with a note that the recording may be cut short. No automatic rejoin in v1.
- Hard cap: leave after `max_call_minutes` (default 240) with `LeaveReason::MaxDuration`.
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
- Proposed: Internal if the team is on one Workspace domain **(pending, tied to the identity decision)**.

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
| Alone 2 min | `Session` | Transcribe |
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
   denied, opt-out, chat off, removal, 30-minute recording, Vietnamese and English speech. Then
   `scripts/smoke-join.sh` runs daily on the server with `MEET_TEST_URL`.

The fixture tests prove our code works against our model of the page. Only the live check proves the
model matches Meet; that gap is accepted and called out in each PR.

## 8. Risks

| Risk | Likelihood | Effect | Mitigation |
|---|---|---|---|
| Meet page changes | High, ongoing | Joins fail | One selectors file, ARIA first, named failure steps, daily smoke test, upload fallback |
| Automation detected, account challenged or suspended | Medium *(unverified)* | No joins until re-login | Visible (non-headless) Chromium under a virtual display on the server, persistent profile, real account, low volume, no scripted sign-in |
| Google terms on automated access | Real *(not legal advice)* | Account suspension | Internal use only, dedicated account, Media API as the long-term replacement |
| Workspace settings block the account | Medium | Knocks or denied | Same-domain account; `Denied` reported clearly |
| Mixed audio only | Likely | Weaker speaker labels | Active-speaker samples; diarization later |
| Chromium plus whisper on one GPU server | Medium | Memory pressure | `max_concurrent_calls`, transcription runs after the call |
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
| 1 | `docs: plan the Meet notetaker` (this) | — | Approved by the maintainer |
| 2 | `feat(notetaker): add denied, removed and max-duration leave reasons` | 1 | `Session` tests for each new reason |
| 3 | `feat(notetaker): add the call driver trait and a scripted fake` | 2 | Fake replays events; trait carries audio frames and speaker samples |
| 4 | `feat(server): run the notetaker loop for scheduled meetings` | 3 | Fake-driver tests from section 7.2, including restart recovery and the concurrency cap. Full workspace tests (touches meeting status) |
| 5 | `feat(core): store encrypted Google refresh tokens` | 1 | Migration, round trip, wrong key fails, hard delete. Full workspace tests |
| 6 | `feat(server): connect and disconnect Google Calendar` | 5 | Mock OAuth tests: scope granted, scope unticked, revoke on disconnect |
| 7 | `feat(server): sync calendars with stored tokens` | 6 | Mock calendar tests: sync, refresh, `invalid_grant` disconnects |
| 8 | `feat(notetaker): launch Chromium and clean up its processes` | 3 | Launch, connect, kill on drop; profile copy deleted |
| 9 | `feat(notetaker): capture call audio in the page` | 8 | Tone fixture test (section 7.3) |
| 10 | `feat(notetaker): join a Meet call and post the notice` | 9 | Fixture states for lobby, admit, denied, chat off |
| 11 | `feat(notetaker): follow the call and leave` | 10 | Fixture states for removed, ended, participant count, chat opt-out, active speaker |
| 12 | `feat(server): add notetaker login and logout commands` | 8 | Profile created with 0700; logout removes it |
| 13 | `feat(core): label speakers from call samples` | 4 | `label` job assigns names from `speakers.jsonl`; retries do not duplicate |
| 14 | `feat(web): connect calendar and notetaker status on Recording & privacy` | 6, 12 | Matches the prototype per `docs/ui-parity.md`, both skins, mobile |
| 15 | `chore(deploy): add Chromium, a virtual display and the secret key` | 12 | Unit file and `docs/deploy.md` updated |
| 16 | Live check with the maintainer, no PR | 4, 7, 11, 13, 15 | Section 7.6 checklist passes; notes added to `docs/review-notes.md` |

New dependencies, each with its reason in its PR: a DevTools protocol client (`chromiumoxide`,
*unverified that it is still maintained; fallback is a small hand-written client over
`tokio-tungstenite`*) in PR 8, `chacha20poly1305` in PR 5.

## Open questions for the maintainer

1. Notetaker identity: dedicated Workspace account, free Google account, or guest? (proposed: Workspace)
2. Is the team on one Google Workspace domain, so the OAuth consent screen can be Internal?
3. Leave when the chat notice cannot be posted? (proposed: yes)
4. External calls (other companies' Meet links): in scope for v1? They will always knock.
5. Retention of the Chromium profile in backups (proposed: excluded).
