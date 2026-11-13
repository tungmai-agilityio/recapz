# Review notes

Running record for the maintainer: assumptions made while building, open issues, blockers and questions.
Add a line when you decide something non-obvious. Newest entries at the bottom of each section.

## Assumptions
- Repo is the private `tungmai-agilityio/recapz`; commits use the `tungmai-agilityio` account via `GH_TOKEN`.
- Branches are `<type>/<name>`, commits and PR titles are Conventional Commits, PRs are squash-merged (CONTRIBUTING.md).
- CI is removed for now (#6). Checks run locally: `make lint test`, `scripts/check-commits.sh`, `scripts/pr-size.mjs`.
- License: proprietary, all rights reserved; no LICENSE file.
- Rust is pinned to 1.99.1 (`rust-toolchain.toml`); web uses pnpm 12.15.0 and Node LTS.
- Week 1 was split into small PRs; PR #1 is the original single-PR version and is closed unmerged.
- A meeting is visible to its owner, or to everyone in the workspace when `share = 'workspace'`. Attendee-based sharing arrives with calendar sync.
- Job runner only claims job kinds it has a handler for; uploads queue a `transcribe` job that waits until the speech work lands.
- Sign-in uses Google OAuth code flow plus the userinfo endpoint (not ID-token verification) and requires a verified email.
- `POST /auth/dev-login` exists for local development and is off unless `dev_login = true` in `config.toml`.
- Week 2 transcription: an upload's `transcribe` job decodes the audio, runs the recognizer and replaces the meeting's segments in one transaction, then sets the meeting to `ready`. Labelling, report and indexing stages arrive in later weeks, so `ready` is reached straight from transcribing for now.
- Without a speech model (or a build without the `whisper` feature) the transcribe job fails and retries instead of storing text. The default model path is `./models/ggml-large-v3-turbo.bin`; `large-v3-turbo` is an assumption until the comparison in `docs/speech-model-comparison.md` has real recordings.
- Meeting detail, transcript and audio reads share one access rule and answer 404 (not 403) for meetings the member cannot see.
- Audio is streamed with single-range support; multi-range requests get 416.
- The `eval` crate is the speech/WER tool; prompt evaluation (`cargo run -p eval` in CLAUDE.md) will need a subcommand of its own when prompts land.
- Zoom v1 join path: Meeting SDK bot, proposed in `docs/zoom-join-path.md`; needs the maintainer's confirmation and a Zoom developer account.
- LLM adapters (#42-#45): `crates/llm` has a `Provider` trait with one non-streaming `complete`; streaming is added with Ask. Prices are micro-USD per million tokens, set per route. One `[llm]` route serves all report tasks (see the report job note).
- Report prompts are version 1 and written without an eval set; replies are checked by shape only (no ISO-date or range checks).
- Report job (#48): the model sits behind a `ReportModel` trait in core so core has no network code; `LlmReportModel` in the server runs the four tasks in turn on one `[llm]` route (per-task routing is deferred). A transcribe job queues the report job only when `[llm]` is configured; otherwise a meeting goes straight to `ready` as before.
- A job that fails for good sets its meeting to `failed` with the error text (first 500 characters).
- `GET /api/meetings/{id}/report` returns `null` until a report exists; a hidden meeting is 404. The Report page polls every 3 s while the meeting is `reporting`.
- Notetaker logic is browser-free: `Session` (lobby 10 min, alone 2 min, call ended, opt-out from anyone but the notetaker itself) and `WavWriter`. A driver must feed it participant samples about every 250 ms and send `from_self` correctly on chat events.
- Opt-out deletion is `purge_meeting` (audio folder first, then rows; search rows go through the chunk delete trigger). It also deletes `llm_usage` and `jobs` rows for that meeting.
- Calendar sync is Google-only and reads upcoming events in a window; events keep `auto_join` and the booked meeting across re-syncs. The join rule is `manual` by default, `auto` books one notetaker per link.
- Join links are parsed strictly (Meet and Zoom hosts only) and stored normalised, so the same call is never joined twice by one member.
- Zoom v1 uses the Meeting SDK bot per the decision doc in #29; it needs Zoom app review, and upload stays the fallback.
- Report email transport is undecided: assume SMTP behind a trait, nothing is wired yet.
- Hosted speech: an optional adapter (Deepgram first, opt-in per deployment, off by default) is decided in `docs/hosted-speech.md` but not built; local whisper stays the default.

## Open issues
- Google sign-in has not been exercised against real Google credentials.
- `BANNED_NAMES` is not set anywhere, so `scripts/banned-names.sh` skips. The list must be provided outside the repo.
- Dependabot opened major bumps (#12 tower-http 0.7, #13 toml 1.x, #14 @types/node 26). Not reviewed or merged.
- CSRF tokens on state-changing routes: done in #74 and merged to `main` (see weeks 11-12 below).
- PRs #15-#21 are open for review as a stack (#16 → #18 → #19 → #20 → #21; #15 and #17 independent).
- Repo settings still allow merge commits and rebase merges (earlier PRs landed as merge commits); squash-only is not enforced. Branch protection needs a paid plan for private repos.
- The `whisper` feature was never compiled: cmake is not installed on the build Mac. Run `cargo check -p recapz-speech --features whisper` where cmake exists; calls were only checked against the whisper-rs 0.14 source.
- Uploads that symphonia cannot decode (notably webm/opus from a browser recorder) make the transcribe job fail. The resampler's ~40 ms delay is not trimmed, and audio under about 1 s makes whisper error.
- If transcription fails for good, the meeting stays at `transcribing` (the job ends `failed`); it is not yet set to `failed` with an error message.
- `GET /api/meetings/{id}/audio`: `bytes=-N` larger than the file returns 416 instead of the whole file. Malformed Range headers also return 416.
- The Report page shows the audio player even before audio is ready, and error text uses a palette colour rather than a token. Any non-401 error shows "Meeting not found."
- `share = 'attendees'` (the column default) is not handled by list, detail, transcript or audio yet; those meetings are owner-only.
- The click-to-seek page and the Report page were checked with jsdom tests and a build only, not in a real browser with real audio.
- Root `Cargo.toml` workspace members will conflict between the speech PR (#33) and the eval PR (#40); keep all crates.
- The test-runner agent is not allowed to run `pnpm build`; builds were run by hand.
- Neither LLM adapter has been called against a live API. The OpenAI-compatible adapter records 0 tokens when a local server omits `usage`.
- `report::generate` returns early with no usage rows if a provider call errors on the retry, so a failed generation's first call is not recorded.
- Assigning the PR author as assignee/reviewer through the API was denied by the permission classifier, so week 3-5 PRs have no assignee.
- The report email to attendees is not built (no mail transport chosen; needs a provider decision and attendee emails from calendar sync).
- Nothing calls `purge_meeting` yet; the driver must call it on `LeaveReason::OptOut`, after checking the caller may delete.
- Deleted text can stay in SQLite free pages and the WAL until a checkpoint or vacuum; decide whether opt-out must vacuum.
- `purge_meeting` assumes audio lives in `audio/<meeting id>/`; it never reads `meetings.audio_path`. A recording stored elsewhere would survive a purge.
- `report::generate` and `generate_report` lose usage rows for calls made before a failure, so spend on failed reports is not tracked.
- Report prompts have no eval set; add one before any v2 prompt change.
- `ROLLBACK`/`COMMIT` failures in the job handlers and `purge_meeting` can leave a transaction open on the connection (same pattern everywhere).
- The report endpoint turns malformed JSON columns into empty lists without a log.
- The Report page shows nothing for a `failed` meeting without a report and nothing when the report fetch fails.
- `join_rule` is stored but no sync caller reads it yet, and the settings API for it is deferred.
- The join API dedupes per member only; two members pasting the same link each get a notetaker. Cross-owner dedupe is not done.
- A second member who can see a call has no access to the meeting a first member booked until it is shared.
- Google Calendar responses are capped (#82, merged), but the HTTP fetch itself is not wired, so the cap is only enforced where `parse_page` is called.

## Weeks 11-12 assumptions (hardening)
- Built (and now merged) on the tip of the Ask stack (`feat/web-open-cited-moment`, #72). Chain, each PR based on the one before: #74 CSRF → #75 Ask rate limit → #77 Vietnamese keywords → #79 streaming in the llm crate → #80 streamed Ask endpoint → #81 embedder swap point. #82 (calendar size caps) is based on #60, because the Ask tip does not contain the calendar server code.
- CSRF is double-submit: `x-csrf-token` must equal the `recapz_csrf` cookie on every POST/PUT/PATCH/DELETE that carries a session, and a browser `Origin`, when sent, must equal `base_url`. The token is random, not bound to the session id; bind it if sibling subdomains ever share cookies. `/api/me` issues the cookie to sessions that began before it existed.
- Ask rate limit is 20 questions per member per minute (`ask_per_minute`), in memory per process, checked after request validation and before retrieval or any model call. A restart resets it.
- Vietnamese: the index already folds marks, so unmarked questions match. Filler is matched with marks only (unmarked forms can be content words). Decomposed Unicode input is kept as one word. Vectors stored for Vietnamese chunks before #77 still include filler and rank slightly lower until re-indexed.
- Streaming: only the Anthropic adapter streams; the OpenAI-compatible adapter delivers its whole reply as one piece through the same interface. The Ask reply is JSON, so the answer text is extracted incrementally. A reply that is invalid JSON is retried without streaming only if no text was sent; after text was sent the call fails with an `error` event.
- Embedder: `Embedder::id()` is stored per chunk (`tenant_003`), search compares vectors only within one embedder, and server start re-queues meetings holding another embedder's vectors. Config `embedder = "hash"` is the only value.
- Calendar caps: 4 MiB body, 250 events, 10,000 scanned characters of notes, 2048-byte links and page tokens.
- Systemd deploy and daily backups landed as #76 and #78.

## Blockers
- No real multilingual embedder is wired. Choosing one and tuning `MIN_SIMILARITY` needs an eval set of real questions and transcripts (the current set is synthetic); the swap point in #81 is ready for it.
- The weeks 11-12 PRs had scoped tests run by the author only; the separate test-runner and code-reviewer agents were not run, so they have had no independent review. The auto-mode permission classifier also blocked adding the review assignee and requested reviewer on the PRs.
- Streaming and the calendar caps have not been run against live provider or Google responses (no keys here).
- #73 (titled as a notes PR) had the Ask branch as its head, so it carried no notes of its own and nothing was lost by not merging it; the week 8-10 notes were never committed anywhere and still need writing.
- Weeks 2-5 merge orders and the `gh pr merge` denial are history: every stack through #82 and the deploy PRs #76, #78 are squash-merged into `main` (see below).
- The Chromium notetaker driver is not built: join flow, platform selectors, audio capture script and the join notice in call chat. It needs a real Meet call and the notetaker's Google account to verify, so it is a live-call blocker, not a code gap I could close here. The pieces that need no browser (session rules, WAV writer, opt-out, speaker matching, purge) are built and tested.
- Zoom Meeting SDK bot needs Zoom app review and an account owner for the app; nothing can join a Zoom call until then.
- Google Calendar needs an OAuth scope for calendar read-only and somewhere to store refresh tokens per member; sync parsing is tested, the live calls are not.
- Real Meet and Zoom calls (and so `scripts/smoke-join.sh`) need test calls kept open and a member cookie; the live join path has not run.
- Report email needs a transport decision (SMTP assumed).
- Weeks 8-12 (#64-#72, #74, #75, #77, #79-#82) and deploy (#76, #78) were squash-merged into `main` in stack order; each stacked branch got a merge commit from `main` first (no force-push), and conflicts with newer `main` code were resolved by hand (runner and main wiring of the report job with the index job and embedder, CSRF token in the join and upcoming tests, report-polling stub in the cited-moment test). #73 and #83 are the notes PRs.
- Weeks 3-7 (#42-#63) were squash-merged into `main` in stack order once the maintainer allowed `gh pr merge`; each stacked branch was merged with `main` first (no force-push).

## Merge order (weeks 11-12)
- Done: #74 → #75 → #77 → #79 → #80 → #81 and #82 are all on `main`..

## Questions for review
- Confirm the Zoom Meeting SDK bot for v1 (#29) and who owns the Zoom developer account.
- Which speech model and Zoom join path to choose depends on real recordings (needs team recordings with consent).

## Meet notetaker loop (assumptions)
- The notetaker claims every `scheduled` meeting that has a join link, oldest first, up to `max_concurrent_calls` minus the calls already `joining` or `recording`. Calendar booking already creates the row about when the call starts, so the loop does not re-check the start time. Revisit if booking moves earlier than one minute.
- Opt-out calls `purge_meeting`, which deletes the meeting row, so no `opted_out` status remains to show. Confirm that is the intended outcome; keeping a stub row would need a purge variant.
- The "may be cut short" note set on a recovered recording is cleared by the transcribe job when it succeeds (it resets `error`). Say so if the note should survive.
- The spawned loop and the Meet driver are not wired yet; `run_meeting`, `claim_scheduled` and `recover` are tested with the scripted driver and called from the loop in the Meet driver PRs.

## Calendar sync (assumptions)
- Sync runs every 10 minutes per connected member for the next 7 days, one member at a time, and starts only when `google.client_id` and `RECAPZ_SECRET_KEY` are set. A malformed key stops the server at start.
- `invalid_grant` deletes the token and sets a "reconnect" marker in the tenant `settings` table (`google_reconnect:<member>`); `GET /api/calendar` returns `needs_reconnect` for it, and saving a new token or disconnecting clears it. Events already synced stay.
- Booking meetings from calendar events (`calendar::book_due`) is still not called anywhere; it belongs in the notetaker loop that PR 11 and later wire up.

## Notetaker browser launch (assumptions)
- Uses `chromiumoxide` 0.9.1 (released and building on Rust 1.85+, so it looks maintained; the hand-written fallback in the plan was not needed). Chromium is started by `std::process::Command` in its own process group, then `chromiumoxide` only connects to its debug port, so cleanup does not depend on the library.
- The process group is killed through `kill(1)` instead of a `libc` call, to avoid one more dependency. Unix only (macOS for dev, Linux for the server).
- The per-call work folder is created with mode 0700 and deleted with the browser. Callers should put it under the data folder (for example `data/notetaker-tmp/<meeting id>`). Backups do not exclude it yet; the deploy PR will exclude `notetaker-profile/` and `notetaker-tmp/`.
- Chromium listens for DevTools on a random localhost port, so any local user on the server can reach a running notetaker browser. Keep the server single-purpose or restrict local logins.
- A server killed without a handler (SIGKILL) leaves its browser and work folder behind; the loop PR should sweep `notetaker-tmp/` at startup.
- `make test-browser` runs the real-browser test; with no Chromium it prints SKIPPED and returns (it does not fail the build).

## Notetaker audio capture (assumptions)
- Capture hooks `RTCPeerConnection` in the page before its own scripts run, so a call page that builds its connections through another constructor (or in a worker) would be missed. The tone test proves the mechanism against our own loopback page, not against the real call page; only the live check can prove that.
- The mix is one stream: if several remote tracks carry the same speaker they add up and the result is clamped. A very loud overlap can clip.
- The capture needs a secure context (https or localhost) and the `--autoplay-policy=no-user-gesture-required` flag, which the launcher now sets.
- Any script on the call page can call the audio binding and so inject fake audio; that is the same trust level as the page's own audio. The server drops payloads over 8 KB unread and drops frames when its queue is full, so a hostile page cannot grow server memory.
- The capture worklet loads from a `blob:` URL; a call page with a strict content security policy for workers could block it (unverified for Meet, checked in the live check). Only the standard `RTCPeerConnection` name is hooked, not `webkitRTCPeerConnection`, and only the top frame is captured.

## Notetaker join flow (assumptions)
- Every label the notetaker looks for on the call page (join and ask-to-join buttons, camera and microphone toggles, leave button, chat button, message box, send button, lobby and denied texts, sign-in link) is in one table in `crates/notetaker/src/meet/page.js` and is our guess at the page. The tests run against our own fixture, so they prove the flow, not the labels. The live check must confirm each one; a wrong one reports `meet_ui_changed: <step>`.
- Typing the notice uses a real text-insert event (not a script that sets the value), then checks the message shows in the chat log before it counts as posted; no message in 3 seconds is `meet_ui_changed: chat_send`.
- Chat disabled (button disabled or an off-notice on the page) is `ChatOff`: the notetaker must leave and never start capture. The caller (the follow step) is responsible for leaving.
- Guest sign-in (typing a display name) is not built: the notetaker uses a signed-in account. A signed-out page is reported as `SignedOut`.
- Camera and microphone toggles are only clicked when the page offers "turn off"; the fake devices in the launcher mean a real device is never opened either way.
- Capture installs its hook before the page loads, so audio frames can flow while the notetaker is still in the lobby. The follow step must write no frames to disk until `join` returns `Admitted` (notice posted), and must leave the call when `join` returns an error.
- A join click that has no effect (the join page still showing after `unknown_limit`) is reported as `meet_ui_changed: join_button`, and the notice only counts as posted when the number of chat messages starting like it goes up.

## Notetaker following the call (assumptions)
- Participant count, who is speaking, the chat log, "removed", "ended" and the rejoin button are looked up in `meet/page.js` by labels we modelled ("N participants", "Name, speaking", "You've been removed from the call", "Rejoin"). Like the join labels they are unverified against Meet; the live check must confirm each. Chat texts from others arrive as the page shows them, sender name included, so the opt-out words match whole words anywhere in the line.
- `MeetDriver` looks at the page every 250 ms and drops audio frames until admission and the posted notice; it returns `NoAudio` after 5 minutes without a frame (the plan's 60-second warning log is not added: this crate does not log).
- Speaker sample times count from admission, while the recording starts at the first frame after it; the gap is a few hundred milliseconds, inside the matching window used by `speaker_for`.
- Browser tests now take a shared lock so only one Chromium runs at a time; with several at once the audio timing test missed its 200 ms tolerance on a busy machine.
- Our own chat lines are recognised by a leading "You:"; a participant named "You Li" is not mistaken for the notetaker. If the real page labels our own lines differently, opt-outs from others would still be seen (they are never flagged as ours) but our notice could be re-read as a member message; the live check covers this.
- The chat line count is clamped when the log shrinks (panel closed and reopened), so an opt-out typed after a reopen is still seen. Lines re-rendered after a reopen can be reported again, which is harmless once a leave decision exists.

## Notetaker login (assumptions)
- `recapz notetaker-login` runs Chromium directly on `data/notetaker-profile/` (mode 0700, tightened if it already existed) with no debug port, so only the person at the window types the password. It succeeds only if the browser exits cleanly and wrote a profile. On Linux it needs `DISPLAY` or `WAYLAND_DISPLAY`; the error tells the person to use SSH X forwarding or VNC. A signed-out session later shows up as `notetaker_signed_out` on the meeting.
- The login window is a separate Chromium instance from calls; calls copy the profile, so a login or logout never disturbs a call in progress, but a call already running keeps its copy until it ends.
- `logout` hard deletes the profile folder and is safe to repeat. Excluding `notetaker-profile/` from backups is applied in the deploy PR.
- `login` only proves the browser exited cleanly and wrote a profile, not that the person finished signing in (Chromium creates the profile on first start), so it says "profile saved" and asks for a test call. `logout` does not check for an open login window; delete it only when no login window is open.

## Labelling speakers (PR 13)
- `speakers.rs` moved from the notetaker crate to core (the notetaker re-exports it), so the `label` job can use `speaker_for` without core depending on a browser crate.
- Sample times count from admission, the same start as the recording, so they line up with segment times. Not checked against a real call.
- The `transcribe` job queues `label` first, then `report` and `index`, only when the call's `speakers.jsonl` exists. Jobs run oldest first, so the report reads named speakers. If `label` itself fails and is retried, the report may already have run with generic names; accepted for v1.
- Each name becomes a `speakers` row with `label` and `display_name` set to the call's display name; members are not matched. Segments with no samples stay unnamed.

## Deploy for the notetaker (PR 15)
- Xvfb runs as its own unit on `:99` and the server unit sets `DISPLAY=:99`; not run on a real host. Chromium's package name and its user-namespace policy differ by distribution, so the docs say what is needed instead of one command for all.
- `/etc/recapz/recapz.env` must exist (the unit reads it); the deploy steps create it empty with mode 0640.
- The backup script skips `notetaker-profile/` and `notetaker-tmp/`. Checked with a throwaway data folder: the recording and the database copies are kept, both cookie files are not.

## Calendar connect on Recording & privacy (PR 14)
- The prototype's Recording & privacy screen needs a settings API that does not exist yet, so this PR adds the screen with only the calendar row; the other rows stay unticked in `docs/ui-parity.md`. The tile copies the prototype's Connections tile (logo, name, one line, badge) inside the screen's boxed row layout.
- The Google callback now redirects to `/recording?calendar=<outcome>` (it was `/`), so the member lands where the outcome is shown.
- Notetaker sign-in status is not shown: the server has no endpoint for it. Add one with the settings API, then a status row here.
- Checked by eye in the in-app browser: Daylight and Ink, mobile width. Dark mode was not checked beyond the shared tokens. Screenshot of this screen and the prototype's are in the thread.
- The server does not serve the built web app yet (no static or fallback route in `crates/server`), so outside `make dev` the redirect to `/recording?...` has no page to land on, same as the earlier redirect to `/`. Serving the app is a separate gap; not part of this plan.

## Starting the notetaker loop (follow-up to PR 4)
- PR 4 built the claim, run and recovery rules but nothing started them. `notetaker_loop` now does, every 15 s: book calendar events that start within a minute (`book_due`), claim scheduled meetings up to `max_concurrent_calls` (counted across tenants), and start each call. Startup runs `recover` for every tenant and deletes `notetaker-tmp/` (left by a killed server) before the loop begins. The loop is off, with a warning, when no Chromium is found.
- The cap is checked across tenants: active calls (`joining`, `recording`) of every tenant are counted before any claim. A tenant that errors during a pass is logged and skipped. A call that ends in an error or panics is settled by `salvage`: a recording with audio keeps it (WAV header repaired, then transcribed, with a note that it may be cut short), anything else fails with `notetaker_error`; only that meeting is touched.
- Each call runs on its own thread with its own current-thread runtime, because `run_meeting` holds a database handle across waits and so cannot be moved between threads. A stuck call cannot stall the server's runtime either. The thread count is bounded by `max_concurrent_calls`.
- A call that cannot start fails the meeting with `notetaker_signed_out` (no saved profile) or `notetaker_no_display` (Linux without `DISPLAY`) and never launches a browser. Any error out of the call itself becomes `notetaker_error` (details in the log, by meeting id only).
- The chat notice is a constant for now ("Recapz notetaker is recording and transcribing this call. Type 'opt out' ..."): the notetaker name and join message settings need the settings API. The name shown in the participant list is whatever the saved Google account is called.
- `max_call_minutes` is converted to ms here; 0 is treated as 1 minute.
- Not tested: a real call through the loop (needs the live check, PR 16); the thread start and the Meet driver wiring are only compiled.
