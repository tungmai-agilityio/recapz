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
- (Superseded by "Speech provider config" below: the server now uses the hosted provider.) Without a speech model (or a build without the `whisper` feature) the transcribe job fails and retries instead of storing text. The default model path is `./models/ggml-large-v3-turbo.bin`. Local whisper is being dropped; see `docs/decisions/hosted-speech.md`.
- Meeting detail, transcript and audio reads share one access rule and answer 404 (not 403) for meetings the member cannot see.
- Audio is streamed with single-range support; multi-range requests get 416.
- The `eval` crate is the speech/WER tool; prompt evaluation (`cargo run -p eval` in CLAUDE.md) will need a subcommand of its own when prompts land.
- Zoom v1 join path: Meeting SDK bot, proposed in `docs/decisions/zoom-join-path.md`; needs the maintainer's confirmation and a Zoom developer account.
- LLM adapters (#42-#45): `crates/llm` has a `Provider` trait with one non-streaming `complete`; streaming is added with Ask. Prices are micro-USD per million tokens, set per route. One `[llm]` route serves all report tasks (see the report job note).
- Report prompts are version 1 and written without an eval set; replies are checked by shape only (no ISO-date or range checks).
- Report job (#48): the model sits behind a `ReportModel` trait in core so core has no network code; `LlmReportModel` in the server runs the four tasks in turn on one `[llm]` route (per-task routing is deferred). A transcribe job queues the report job only when `[llm]` is configured; otherwise a meeting goes straight to `ready` as before.
- A job that fails for good sets its meeting to `failed` with the error text (first 500 characters).
- `GET /api/meetings/{id}/report` returns `null` until a report exists; a hidden meeting is 404. The Report page polls every 3 s while the meeting is `reporting`.
- Notetaker logic is browser-free: `Session` (lobby 10 min, alone 30 s, call ended, opt-out from anyone but the notetaker itself) and `WavWriter`. A driver must feed it participant samples about every 250 ms and send `from_self` correctly on chat events.
- Opt-out deletion is `purge_meeting` (audio folder first, then rows; search rows go through the chunk delete trigger). It also deletes `llm_usage` and `jobs` rows for that meeting.
- Calendar sync is Google-only and reads upcoming events in a window; events keep `auto_join` and the booked meeting across re-syncs. The join rule is `manual` by default, `auto` books one notetaker per link.
- Join links are parsed strictly (Meet and Zoom hosts only) and stored normalised, so the same call is never joined twice by one member.
- Zoom v1 uses the Meeting SDK bot per the decision doc in #29; it needs Zoom app review, and upload stays the fallback.
- Report email transport is undecided: assume SMTP behind a trait, nothing is wired yet.
- Hosted speech and LLM providers (Deepgram, OpenAI default) replace local whisper; decided in `docs/decisions/hosted-speech.md`, not built.

## Open issues
- Google sign-in has not been exercised against real Google credentials.
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

## Next up on Meetings
- Shown as a section above the day list, not an aside: the page has no aside layout yet. It lists the next 5 events from `GET /api/meetings/upcoming` (the server returns up to 50) with day and time, title, platform and the auto-join switch. Not "today only": the API returns all coming events, so the day label shows when it is not today.
- Events with no join link show "No link" instead of a switch, since `PATCH /api/calendar-events/{id}` refuses them (404).
- Calendar not connected shows a hint to Recording & privacy; a connected calendar with no events shows the empty line with the same link. A calendar that needs reconnecting is treated as connected here and its list will just be stale; the reconnect prompt lives on Recording & privacy.
- No "Join rules" link and no paste-link box (own task). The S2 row in `docs/ui-parity.md` stays unticked.
- Tested with a stubbed API only; not run against a seeded scratch database or checked by eye in both skins, because Calendar sync still returns 403 for the maintainer.

## Sidebar footer overflow
- The account menu was anchored to the trigger's right edge, so in the 208px rail it opened leftward off-screen, and the rail (scrolling vertically) grew a horizontal scrollbar. In the rail it now anchors to the left edge (`align="left"`), the email line truncates with an ellipsis, and the rail hides horizontal overflow. In the top bar (900px and under) it still anchors right, and the bar now wraps instead of running past narrow screens.
- A user with no display name now sees their email once in the menu header (it was shown twice).
- Checked by eye in Daylight and Ink with a stubbed API at 1024px and 360px (menu and page inside the viewport, no horizontal scroll). The unit test only asserts the anchor classes; jsdom has no layout. Not checked: Daylight dark / Ink light combinations separately (same classes, colours only), or a real signed-in session.

## Join link in upcoming meetings
- `GET /api/meetings/upcoming` now returns `join_url` (replacing `has_link`) so Next up can show "Open link". It is only the signed-in member's own events, so nobody sees a link they were not invited with. The earlier rule "the link must not reach the browser" is dropped on the maintainer's request; the server test now checks the link is returned and that another member's list has none of it.
- The link only renders when it starts with `https://`. Participants are not included: nothing stores attendees yet.
## Rename a meeting
- `PATCH /api/meetings/{id}` with `{"title": "..."}` returns `{"title": <saved>}`. The title is trimmed, must be 1 to 200 characters (counted as characters, not bytes), else 400.
- Assumption: same permission as delete, owner or admin only (403 for a member who can merely see a shared meeting; 404 for a meeting they cannot see). Unlike delete it is allowed while the meeting is still processing, since the title is not an input to any job.
- The meeting list reads titles from the same row, so the list and Ask citations show the new title on their next fetch; the Report page updates its cache at once and refetches the list.
- Web: Rename sits in the "Meeting actions" menu on the Report page and turns the heading into an inline editor (Enter saves, Escape or Cancel discards, a server error shows under the field). The list page has no row menu, so there is no rename there.
- Not done: the stored search rows (`chunks`) carry no title, so nothing else needs updating. Not checked by eye in the browser or in both skins; tested with a stubbed API and the Rust router.

## Send notetaker on Meetings
- A "Send notetaker" button next to Upload opens an inline form above the list (not a modal: the page has no aside layout and the form needs no focus trap). Fields: meeting link (required, `type=url`) and optional title; Cancel closes it.
- Calls `POST /api/meetings/join` with `{url, title}`; the title is left out when blank. While it runs the submit button reads "Sending…" and is disabled. On success the form closes, a toast says "Notetaker is on its way" and the list is refetched.
- Errors show under the link field with the status prefix removed (400 "not a Meet or Zoom link", other server refusals the same way) and the typed values are kept so the member can fix and retry. No client-side check of the host: the server is the judge of what counts as a Meet or Zoom link.
- Tested with a stubbed API only; not checked by eye in both skins or against a real server. The S2 rows in `docs/ui-parity.md` stay unticked (no side-by-side screenshot with the prototype state yet).

## Attendees on upcoming meetings
- Sync keeps each event's invitees (name, email, reply status) in a JSON `attendees` column on `calendar_events` (migration `tenant_005`), replaced on every sync. One column instead of a table because they are only ever read together with their event, through the owning member's query; a table can come later if invitees need to be searched.
- Google already returns `attendees` for the events list under the existing calendar read scope, so no scope change; the request now also sends `maxAttendees=50`, and at most 50 are kept (names cut at 100 characters, emails at 254).
- Left out on purpose: the calendar owner (`self`), rooms and other resources, and entries without a usable email. An event with no invitees returns `"attendees": []`. Guest lists Google hides from guests simply come back without the field.
- `GET /api/meetings/upcoming` returns `attendees: [{name, email, status}]` per event, only for the signed-in member's own rows, like `join_url`. Emails of other people are therefore shown to someone who already saw them on their own invite.
- Web: Next up shows up to four faces (name, or email when Google sent no name) with a "+N" count; nothing is drawn for an event with no invitees. The reply status is returned but not shown yet.
- Tested with the mocked Google endpoints and seeded databases only. Real sync still returns 403 for the maintainer until the Calendar API is enabled on the Google project, so this has not been run against Google or checked by eye in either skin.

## Deepgram streaming client
- `crates/speech/src/stream.rs`: `DeepgramStream::start(url, key, language)` returns at once; a worker thread connects, sends frames as little-endian bytes and turns `Results` messages into `LiveEvent::Partial` / `Final`. Any failure becomes one `Unavailable` event; later `send_frame` calls return false and never block (bounded backlog of 100 frames, about 10 s, then frames are dropped).
- Blocking `tungstenite` (already in the lockfile via the browser driver) instead of an async client, because the speech crate is synchronous. New direct dependency: `tungstenite` with rustls and bundled roots for `wss://`, plus `serde_json` to read results.
- Not verified against the real service: the query parameters (`model=nova-3`, `language`, `encoding=linear16`, `sample_rate=16000`, `interim_results`), the `Results` message shape, the `multi` language value for vi+en, and the `CloseStream` text message are taken from memory of the provider docs. Only a mock server on localhost was tested; the `wss://` path and the real key header were not exercised. Not wired into the server or live view yet.
- No connect or handshake timeout yet: a black-holed host never emits `Unavailable`; frames just start dropping once the backlog is full, and recording is unaffected. Add a `TcpStream::connect_timeout` plus `client_tls` when the server wires this in. `Unavailable` also has no reason (a rejected key looks like a drop); the fail-fast auth alert needs a reason variant in the config PR. No KeepAlive message, which is fine while frames flow.

## Speaker labels from segments
- `Segment.speaker` is the provider's tag (any string); the transcribe job numbers distinct tags `Speaker 1`, `Speaker 2`, ... by first appearance. A retry deletes the meeting's speaker rows and rebuilds them, like segments, so nothing duplicates. A rename a member made before a retry is lost, as the segments it pointed at are replaced too.
- The `label` job (call recordings) still runs after and replaces speakers by its own names.
- The `whisper` feature path only gained `speaker: None`; it was not compiled here (feature off).

## Deepgram batch transcriber
- `crates/speech/src/deepgram.rs`: `DeepgramTranscriber::new(base_url, key)` posts the whole recording as 16-bit WAV to `/v1/listen` (`nova-3`, `diarize`, `utterances`, `punctuate`) and maps `results.utterances` to segments; the provider's speaker number is the tag the transcribe job numbers. Language `vi` and `en` pass through, anything else (including `vi+en`) uses `multi`.
- 429, 5xx, timeouts and connect errors are retried twice (`Retry-After` honoured up to 30 s, else 2 s then 4 s); other 4xx fail at once. Errors hold the status or failure kind only.
- Uses `reqwest` with its `blocking` feature (already a dependency elsewhere) rather than a new HTTP crate; the trait is synchronous and runs on the job thread. `tempfile` is a dev-dependency for the WAV round-trip test.
- Not verified against the real service: query parameters, the `utterances` shape, the `multi` value, and that `utterances` carry `speaker` for Vietnamese. Mock server on localhost only; the https path and key header were not exercised. A 2 hour file (provider limit) is still unchecked (#226). Not wired into the server yet (#220).

## Speech provider config
- The server reads `RECAPZ_SPEECH_KEY` from the environment only (a key written under `[speech]` in `config.toml` is ignored) and refuses to start without it. `[speech] base_url` exists only so tests can point at a mock. The transcribe job now always uses the Deepgram transcriber; the whisper build feature and its model loader are no longer used by the server (removal is #225). `speech_model` is gone from the config.
- `/api/me` returns `speech_provider: "Deepgram"`, a constant, since there is one provider. A second provider would need a config field and a real lookup.
- Not verified: starting the real binary with a real key, and a real transcription. The start check and the `/api/me` field are covered by unit and router tests only.

## Speech provider on upload and privacy
- Upload's lede ("Processed on your server" was no longer true) now reads "The audio is sent to {provider} to be transcribed." Recording & privacy gets a "Where audio goes" card. Both read `speech_provider` from the Shell's existing `/api/me` query and fall back to "the speech provider" while it loads or if the server does not send it.
- The card says only that audio goes to the provider live and from the recorded file. It does not claim no-training or no-retention, because those account settings are still unverified (`docs/decisions/hosted-speech.md`).
- Checked in a real browser (Chrome, Vite dev server with the API mocked) at 1280 and 390 px, Daylight light only; not Ink or dark, not against the real server, no prototype side-by-side.

## Recordings in object storage (first part of #222)
- Optional `[storage]` (endpoint, region, bucket) plus `RECAPZ_STORAGE_KEY_ID` and `RECAPZ_STORAGE_SECRET` from the environment only. Empty bucket keeps today's local-disk behaviour; a bucket with a missing field fails at start.
- After a transcribe job succeeds the runner queues an `archive` job (only when a store is configured). It uploads `audio/{id}/recording.wav` to `{tenant}/audio/{id}/...`, sets `meetings.audio_remote = 1` (migration `tenant_007`, additive), then removes the local audio file. `speakers.jsonl` stays for the label job. A failed upload keeps the local file and retries; a rerun is a no-op.
- Playback: a remote recording is served through the same route and the same meeting access check. Ranged requests become ranged GETs, capped at 8 MiB per response (a player asking `bytes=0-` gets 206 with the first 8 MiB and asks again); a plain request streams the whole object. The bucket is never exposed.
- Delete and opt-out: `purge_meeting` deletes the object first, then local files and rows. A remote recording with no store configured is an error, never a silently kept object. The notetaker's opt-out during a call passes no store because nothing is uploaded yet.
- Retranscribing is not possible once the local file is gone (the transcribe job fails with "meeting has no audio"). Fine for v1; a re-fetch step would be needed.
- Dependency: `rust-s3` (0.38, `tokio-rustls-tls`, shares the `reqwest` 0.12 already in the tree) for SigV4 signing and S3 requests, instead of hand-rolled signing or the much larger AWS SDK.
- Tested against a mock S3 server on localhost (put, multipart put of a 20 MiB file, head, ranged get, delete), which does not check signatures. Not verified against any real provider (the provider is undecided): request signing, path-style addressing, its multipart behaviour, and `HEAD` content length. Not built yet, left for the next PR: audio retention (default 90 days, admin-settable) and the privacy-screen line naming object storage.

## Audio retention (second part of #222)
- `settings.audio_retention_days` (tenant settings table, default 90, 0 keeps audio forever, at most 3650). Admins change it with `PUT /api/settings/audio-retention`; any member can read it with `GET`. The Recording & privacy screen shows it; there is no select for it yet (no settings screen API beyond this).
- The runner sweeps once at start and then hourly. Past the period, a `ready` or `failed` meeting's audio is deleted (the object first, then the local file), and `audio_path` is cleared; transcript, report and search rows stay. Meetings still recording or processing are never swept. Remote audio with no store configured is skipped, not forgotten. The sweep uses `ended_at`, falling back to `created_at`.
- The player and the audio route treat a cleared `audio_path` as "no audio" (404). The report page's player was not checked for how it looks when the audio is gone.
- `/api/me` reports `recording_storage` (`object_storage` or `local_disk`); the privacy card reads it. Checked by eye in Chrome at 1280 px, Daylight light only, API mocked.
- Not verified: a sweep against a real bucket (mock only).
- Known limits, from review: one failing object delete aborts that tenant's sweep (and the runner moves on to the next sweep an hour later), so a single stuck object delays the rest; per-meeting error handling is the fix. `PUT` maps every settings error to 400. Only the audio file is removed, not the whole `audio/{id}` folder. The sweep's retry path after a failed delete has no test.

## Deepgram request parameters (fix)
- Checked against Deepgram's docs on 2026-11-18: `diarize` is deprecated (it forces the v1 diarizer) and a request that sets both `diarize` and `diarize_model` is rejected, so the batch request now sends `diarize_model=latest` only. `mip_opt_out=true` (excludes the request from the model improvement program) is sent on both the batch and streaming requests. Mock-server tests assert both, and that `diarize=true` is absent. Still unverified against the real service: that `diarize_model=latest` works with `language=multi`, and Vietnamese diarization (#226).
- Cost to use in #223 (second-hand, from Deepgram's pricing pages on 2026-11-16, not verified by us): Nova-3 multilingual batch about $0.0058/min (promo, can change) plus diarization about $0.0020/min, so about $0.47 per meeting hour with speaker labels, not the $0.31 in `docs/decisions/hosted-speech.md`. Limits to keep in mind: 2 GB per file, and a pre-recorded request running past about 10 minutes can return 504, which matters for long meetings in batch mode (retries and chunking).
- Language: The maintainer's decision was a per-meeting language setting (vi, en or multi, default vi). Not implemented: the upload API reads no language, `meetings.language` defaults to `vi+en`, and both transcribers map anything other than `vi`/`en` (including `vi+en` and `auto`) to `multi`. So every meeting today uses `multi`. Needs a ticket: add a language field on upload and the join flow, default `vi`, and map `multi` explicitly.
- The old plan's fallback to local whisper stays dropped (no fallback provider), per the decision record.

## Usage and monthly cap (#223, API part)
- `speech_usage` (migration `tenant_008`): one `batch` row per meeting with the audio seconds and an estimated cost, written by the runner after a transcribe job stores its transcript (audio length = the last segment's end; billing is by audio duration, so this is an estimate). A failure to record it is logged, never fails the job (a retry would pay for the transcript again). Language-model usage was already in `llm_usage`; Ask rows have no meeting.
- Rates are config: `[speech] price_per_minute_micro_usd` (default 7800 = batch plus speaker labels, second-hand list prices that can change) and the LLM prices that already exist. The cap is `monthly_cap_usd` (0 = off), compared with the UTC calendar month's estimated speech plus language-model cost.
- At the cap the runner stops claiming paid job kinds (transcribe, report, index, document ingest); archive and label still run, and queued paid jobs resume next month or when the cap is raised. Stream usage is not recorded: there is no live view yet.
- `GET /api/usage` (admin only) returns the month's totals, the cap, `paused`, and the 20 costliest meetings. **Not built yet:** the admin alert and per-meeting cost on screen; they need a web PR (next). Nothing logs or notifies when the cap is first reached.
- Not verified: the estimate against a real Deepgram or provider invoice.
- Per-meeting cost follows the same access rule as every meeting read: an admin sees titles and costs only for meetings they own or that are shared with the workspace; the month totals count every meeting. Deleting a meeting removes its usage rows, so a purge lowers the reported spend (and can lift a pause), same as `llm_usage` already did.

## Usage card (web part of #223)
- Recording & privacy shows admins a "Usage this month" card: estimated total, cap (or "no cap set"), speech and language-model split, a red alert when the cap is reached, and the cost per meeting (up to 20, only meetings the admin may see). Members never call `/api/usage`. Amounts are estimates from list prices, said so on the card.
- Checked in Chrome at 1280 px (Daylight light) with the API mocked, not Ink, dark, a phone width of this card, or a real server. Nothing notifies an admin outside this screen when the cap is first reached (no email or banner elsewhere).

## Per-meeting language (#245)
- A meeting's language is `vi` (default), `en` or `multi`. Upload takes a `language` multipart field and `POST /api/meetings/join` a `language` JSON field; anything else is a 400, never silently another language. Calendar auto-join meetings are created as `vi`. The upload screen has a select (Vietnamese, English, Mixed languages); the Send notetaker form on Meetings has no language control yet (it sends none, so those meetings are `vi`).
- `deepgram_language()` maps `vi`, `en` and `multi` explicitly, and `vi+en` and `auto` (what meetings made before this carry) to `multi`. An unknown value fails the batch job with "unsupported meeting language" and makes the stream report unavailable, without contacting the provider. Existing meetings keep their stored value and keep working.
- The schema default for `meetings.language` is still `vi+en` (a column default cannot be changed without rebuilding the table); every insert path now sets the language explicitly.
- Still unverified (#226): `language=multi` with Vietnamese plus English, `vi` alone, and diarization on Vietnamese.

## Whisper and CUDA removed (#225)
- Gone: the `whisper` and `cuda` features in `recapz-speech` and `recapz-server`, `whisper-rs` and its cmake/CUDA build, the whisper transcriber, the language window helper only it used (and its test), `--features whisper[,cuda]` in the Makefile, and the whisper, CUDA and cmake lines in `docs/deploy.md`, `docs/running-locally.md`, `docs/specs/live-ask.md` and `docs/specs/meet-notetaker.md`. `speech_model` left the config in #220. The `Transcriber` trait, `StubTranscriber`, `is_non_speech` and upload decoding stay.
- Not changed on purpose: `docs/decisions/` (the decision record and the superseded model comparison are history) and this file. `CLAUDE.md` still says `make release` builds "with CUDA": it is a guard-rail file, so that one line is a separate PR left for the maintainer.
- Verified: `make -n release` shows a plain `cargo build --release`; the workspace builds, tests and lints without the feature. Not run here: a real `make release` (full release build).

## Single-VPS deploy readiness (2 vCPU / 4 GB; written for Hetzner Cloud Singapore, host is now a DigitalOcean Droplet in San Francisco)
- Gap found: the binary served no web app (no static serving despite "embedded" in the docs); fixed by #281. Also found: the LLM key was read only from `config.toml` while the docs said env file (#283); #268 (TLS crypto provider) must be merged or a release binary panics at its first TLS call.
- Prepared (PRs #281 to #286, unmerged when written): web embedded in the binary, Caddyfile and env template, `RECAPZ_LLM_KEY`, `scripts/deploy-release.sh` (release symlink, health check, rollback), `scripts/smoke-deploy.sh`, `scripts/provision-host.sh`.
- Assumptions: (1) Debian 12, not Ubuntu, because Ubuntu's `chromium` is a snap; (2) the binary is built off the host, since a release build on 4 GB RAM is unmeasured and likely too tight; build on the same Linux/CPU architecture (x86_64 assumed; a macOS build cannot be copied); (3) 90-day recording retention is the app's own sweep (`audio_retention_days`, default 90, admin-settable), not a bucket lifecycle rule; (4) no object storage provider or region is chosen or verified (see `docs/decisions/hosted-speech.md`).
- Not built: offsite push of `/var/backups/recapz` (the plan had restic to a Storage Box; recordings are in object storage now, but the SQLite backups still live only on the host disk), and the live Meet check / `join_button` fix from the old plan. Not verified: provisioning on a real host, Caddyfile validation, Chromium under the unit's hardening, Meet from a datacenter IP, a real Deepgram or bucket call (#226).

## Hetzner replaced by DigitalOcean in the deploy docs
- Docs and example-file comments only; no scripts, workflows or code changed. Hosting is a DigitalOcean Droplet in San Francisco (Debian 13, 2 vCPU / 4 GB, 79 GB disk), `tungmai.com` on Cloudflare with a DNS-only A record, Caddy for HTTPS, deploys through the `release-build` workflow `deploy` job.
- Assumptions: recordings are on server disk (no bucket enabled) and the object storage provider is undecided; Droplet backups are not enabled; the Debian 13 run is as reported by the owner, not checked from this repo; `scripts/provision-host.sh` only requires `ID=debian`, so it may run on 13 but that is untested here.
- Not changed: the 2026-11-18 history line and Singapore wording in the older sections above stay as a record of the earlier plan.

## Tauri shell (#344)
- `desktop/` is its own Cargo workspace; the root workspace lists `crates/*` only, so CI never builds it. `make desktop-check` (fmt + Clippy) is the only gate; the pre-push hook runs it when the pushed commits touch `desktop/`.
- `base_url` is `RECAPZ_BASE_URL` at build time (default `http://localhost:8080`, trailing slash trimmed). `build.rs` bakes it in and writes the gitignored `capabilities/base-url.json` (`remote.urls = [base_url/*]`, no permissions yet; the capture tickets add the audio commands).
- Sign-in: the shell cancels navigation to `/auth/google` and opens `/auth/desktop` in the system browser with `open` (macOS only, no opener plugin). The `recapz://signed-in?code=` handler runs the exchange `fetch` inside the page (same origin) and reloads.
- Placeholder icon (generated flat PNG) serves Dock, tray and bundle; swap for the real mark later. The tray icon is not a template image, so it will not tint in dark menu bars.
- Not verified: a real launch, the sign-in round trip and the deep link (they need a bundled `.app`; `cargo run` does not register `recapz://`). Only `cargo build` and Clippy ran. Unverified assumption: the webview's `fetch` carries the `Origin` header the exchange requires.
- Spec status moved from draft to agreed (2026-11-23).
- Review follow-ups: the exchange now waits for a page load on the app's own origin (a pending code, run by the page-load hook) and reloads only when the exchange succeeds; `build.rs` creates `capabilities/` itself (a fresh clone has none); the pre-push gate fails closed when it cannot diff the pushed range.
- Open decision, not changed here: the deep link carries no app-side `state`, so a web page could open `recapz://signed-in?code=` with a code from the attacker's own sign-in and leave a member in that session (login CSRF). Fixing it needs a server change (bind the code to a value the app generated before opening the browser); suggest a follow-up ticket after #343.
- Not done: a CSP for the remote webview, and handing links outside `base_url` to the system browser.

## System audio tap (#345)
- Global Core Audio process tap (`initStereoGlobalTapButExcludeProcesses([])`, unmuted, private) inside a private aggregate device with the tap as its only sub-device, no output device. Frames: downmix to mono, box-filter resample to 16 kHz, round to i16, cut into 1600-sample frames, sent as raw little-endian bytes (3200 per message) over a `Channel<InvokeResponseBody>`.
- The resampler and framer live in `audio/frame.rs` (not `tap.rs`, as the ticket listed) so the pure logic is unit tested apart from the macOS calls and the mic ticket can reuse it. The tests assert exact sample counts (one second at 48 000, 44 100 and 16 000 Hz gives 10 frames).
- `audio_permission` tries to create a tap and reports `"granted"` or the OSStatus error. Assumption: a denied grant fails the create or yields silence; macOS has no public preflight for audio capture, so a silent tap would not be caught here.
- The audio callback locks a mutex and allocates; fine at 100 ms-scale buffers, revisit if the 30-minute hidden-window test shows drops.
- Capability: `build.rs` now registers the three commands as an app manifest, and the generated `capabilities/base-url.json` grants only `allow-audio-start/stop/permission` to `base_url/*`.
- Review fixes: a listener on the tap's format property re-reads the sample rate (the callback rebuilds its framer when it changes); a non-float format is refused at start; `audio_start` replaces a stale session, and a page load or window destroy stops capture, so a lost page never leaves the tap and App Nap hold running; the commands are async. The recording holds `UserInitiated`, which also blocks idle system sleep, on purpose. Not done: channel count is read once (only the rate is tracked), and the callback still allocates and sends on the audio thread.
- Not verified (no mic/audio grant or call on this machine): a real tap, the permission prompt, a Zoom call, the AirPods switch (a gap over 2 s is untested; the tap has no output-device binding by design), App Nap with a covered window, the rate listener (`None` dispatch queue) firing, and whether `cargo run` embeds `Info.plist` (a bundled build is the safe check). Only `cargo build`, Clippy and the unit tests ran.
- Open: the deep link carries no app-side `state` (filed as its own ticket).

## Microphone capture and mix (#346)
- `audio/mic.rs` opens the default input device through the Core Audio HAL (an IOProc on the device, like the tap) and re-reads its rate when it changes. A listener on the default-input property hands a reopen to a worker thread (tearing a device down inside a Core Audio listener can deadlock). `Mic::start` fails with "no microphone found" when there is no input now; a reopen after a swap is retried 5 times, 200 ms apart, then the mic stays silent until the next change.
- `audio/mix.rs` is a sum-and-clamp (saturating add) mixer over two 1600-sample frame streams. A frame goes out when both sides have one, or when either side is two frames ahead (the other side counts as silence), so a stalled mic or tap delays the other by at most one frame instead of growing a buffer. The tests assert exact frame sizes, sums, clamping at both ends, and the silence rule.
- `audio_start(frames, mode?)` and `audio_permission(mode?)`: `mode` is `meeting | in_room` and is optional (default `meeting`), so the page code written against #354 keeps working. `in_room` skips the tap and mixer and sends mic frames directly.
- `audio_permission` now returns `"granted"` or a comma-separated list of what is missing: `microphone`, `audio_capture`. In-room only checks the microphone. Assumption: a failed tap creation means audio capture is not granted (the OSStatus text from #345 is no longer returned). Microphone status uses `AVCaptureDevice authorizationStatusForMediaType:` ("soun") and asks once when undecided, so the first call can show the prompt and waits for the answer. This links AVFoundation directly with `objc2` message sends instead of adding a crate.
- Review fixes: a sample rate or channel count of 0 (a device mid-switch) is refused at open and ignored by the listener, and `Resampler` no longer loops forever on rate 0 (tested; the tap had the same hole); `Mic::start` checks microphone permission itself so a page that skips `audio_permission` cannot record one side silently.
- Shared code moved out of `tap.rs` so both captures use it: `Awake` (the App Nap and idle-sleep opt-out, now also held by in-room), `mono_of` (input buffers to mono). `NSMicrophoneUsageDescription` added to `Info.plist`. New dependency: `serde` with `derive`, already in the lock file through Tauri, for the `mode` argument.
- VoiceProcessingIO spike: not run. It cannot be judged without a live Zoom call, which this machine does not have, and the ticket adopts it only if it cancels Zoom's output. Not adopted, so the dialog keeps recommending headphones. Left for a hands-on check; adopting it would change the mic open path only.
- Hardened runtime: the app is signed with a self-signed certificate and no entitlements file exists. If hardened runtime is ever enabled, `com.apple.security.device.audio-input` is needed for the mic.
- Not verified (no mic grant, no call): a real mic capture, the permission prompts and the "which one is missing" answers, the device swap (AirPods, wired headset) and the rate listener, and the mixer on live audio. Only `cargo fmt`, Clippy and the unit tests ran.

## #347 Record dialog in the desktop app

- Detection is `window.__TAURI__`, which only exists with `withGlobalTauri`; it was not set, so `tauri.conf.json` now sets it (one line). The capability still limits IPC to `base_url` and the three audio commands.
- `web/src/capture/native.ts` runs the native capture: permission check, `audio_start` with a `Channel`, frames to the existing `FrameSocket` (60 s buffer unchanged), `audio_stop` on stop or discard. "Meeting app" sends `meeting`, "In-room" `in_room` (the server `mode` stays `tab`/`room`).
- Assumption: the native side sends no levels, so the page takes the peak of each 100 ms frame and shows one "Audio" meter.
- Not done, needs Rust: the menu-bar red dot (no command to set it) and the "system audio stopped" signal (no native event for it). The warning text is wired to `tabEnded` and shows "System audio stopped" once a native event sets it. Both belong in a small follow-up.
- Not verified: recording in the running app, a Zoom call and a Safari Meet call with headphones, and the 30-minute minimized-window run. Only `pnpm test`, lint and the type check ran.

## #348 Local macOS build and release attach
- `scripts/desktop-cert.sh` creates the "Recapz Local" self-signed code-signing certificate in the login keychain (idempotent, 10 years, code-signing usage). It uses `/usr/bin/openssl` because Homebrew OpenSSL 3+ writes a PKCS#12 that `security import` rejects. The cert shows as `CSSMERR_TP_NOT_TRUSTED`; that is expected for self-signed and does not stop `codesign`.
- `make desktop-dmg` runs `cargo tauri build` in `desktop/src-tauri` with `APPLE_SIGNING_IDENTITY="Recapz Local"`, copies the `.dmg` to `./Recapz.dmg` (gitignored), and verifies the `.app` signature. Assumption: `RECAPZ_BASE_URL` defaults to `https://recapz.tungmai.com` (the host in `docs/deploy.md`), overridable with `DESKTOP_BASE_URL`. It needs `tauri-cli` v2 installed once (`cargo install`, not a repo dependency).
- Release skill: new step 7 runs the build and `gh release upload <version> Recapz.dmg`, skipped off the maintainer's Mac.
- Not verified: the cert import was tested against a temporary keychain only. `cargo tauri build` with the self-signed identity, the signed `.dmg` itself, the TCC grants surviving a second build, and installing on a clean macOS 14.2 machine were not run.

## #353 Desktop sign-in state
- The app generates a 32-hex `state` (from `/dev/urandom`, no new dependency) when the sign-in button is pressed, keeps it as the one pending state, and opens `/auth/desktop?state=…`. The server keeps it in the `recapz_desktop` flag cookie (was `1`), so it survives the Google round trip with no new store, binds it to the one-time code, and returns it as `recapz://signed-in?code=…&state=…`. `POST /auth/desktop/exchange` takes `{code, state}` and refuses a different or missing state.
- The app ignores a link whose state is not the pending one and clears it on first use, so a replayed link does nothing. A second press of the sign-in button replaces the pending state; the first browser tab's link is then ignored.
- Assumptions: `/auth/desktop` without a well-formed state (16 to 128 of `A-Za-z0-9_-`) is refused, so an old app build cannot sign in until updated (the only build is local and unreleased). A wrong-state exchange still uses the code up, so a code cannot be guessed against. The spec's pointer to the deep link format is unchanged apart from the extra `state` parameter.
- Not verified: a real round trip through Google and the `.app` deep link (needs a bundled app, as in #344).

## #358 Menu-bar recording dot and system-audio-stopped event
- `recording_state(on)` swaps the tray icon between the app icon and a copy with a red dot painted at runtime (`dot.rs`), so there is no second image asset. The web recorder calls it after a capture starts and when it ends (stop, discard, or the server closing it); the shell also clears it when the page reloads and when the window is destroyed, next to the existing `audio::stop()` calls. Only `allow-recording-state` joins the base-url capability.
- "System audio stopped" has no Core Audio failure callback to hook here, so a watchdog (`audio/watch.rs`) treats 3 s without a tap buffer as stopped or failed. This assumes a running tap delivers buffers even for silence; if it does not, a silent Mac would show the warning falsely, and the threshold or the signal (for example an `IsAlive` listener on the aggregate device) needs revisiting.
- The page cannot listen for Tauri events (the capability allows commands only, by the ticket), so the shell runs `window.dispatchEvent(new CustomEvent('recapz-system-audio-stopped'))` in the page, the same way the sign-in exchange runs its `fetch`. The recorder only listens in meeting mode.
- Not verified: the red dot on a real menu bar (light and dark), the dot clearing on reload and window close, and the warning on a real tap failure (unplugging or switching output, quitting coreaudiod). Unit tests cover the dot painting and the stall logic, and vitest the page side.

## desktop-dmg checks for the Tauri CLI
- `make desktop-dmg` now fails early with the install command when `cargo tauri` is missing (it used to fail with cargo's "no such command: `tauri`"). Assumption: a check with a clear message is enough; no separate `desktop-tools` target, since the install is one command already documented in `docs/deploy.md`. The version range `^2` matches the `tauri = "2"` line in `desktop/src-tauri/Cargo.toml`.
- Not verified: the full build; the check was run with and without the CLI on PATH only.

## desktop blank window and dmg check
- Blank window: the window opened `RECAPZ_BASE_URL` directly, so an unreachable server (recapz.tungmai.com did not answer on ports 80 or 443 from the maintainer's Mac when checked) left a white window with no message. Assumption: that is the cause; nothing else in the config (CSP is null, the remote capability matches the baked URL, `frontendDist` exists) blocks the page, and PR 360 only touches the sign-in flow. The window now opens the bundled `ui/index.html`, which probes `/api/health` and then switches to the server, or shows "Can't reach" with a retry button.
- `make desktop-dmg`: Tauri deletes `bundle/macos/Recapz.app` after making the DMG, so the build now passes `--bundles app,dmg`; the `.app` stays and `codesign --verify` has something to check. Verified: both `.app` and `.dmg` exist after that build.
- Verified by the maintainer: a local build pointed at a plain-http test server showed that server's page, so the probe and the switch to the server work. Not verified: the "Can't reach" page on screen, a live https server, and the signed build with the "Recapz Local" identity.

## desktop close button and quit
- Cause: the window's close handler always called `prevent_close` and minimized, so the red button never closed anything. The spec said "Close minimizes", which is what Tung reported as broken, so the spec line now says close closes.
- Assumption (spec was silent on recording): with no recording the window closes and the app stays in the Dock and menu bar (Dock click reopens it); while recording, a dialog asks "Stop and close" or "Keep recording". Quit (menu bar, Cmd+Q, tray Quit) always exits, and `RunEvent::Exit` stops the capture. New dependency `tauri-plugin-dialog` for that dialog.
- Not verified: clicking the button in a running app and Cmd+Q; only fmt and Clippy ran (no test is feasible for window events).

## Desktop permission link
- The permission error in the app has an "Open System Settings" button. It runs `open x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone` (or `Privacy_AudioCapture` for the system audio). Assumption: `Privacy_AudioCapture` lands on the System Audio Recording section on macOS 14.4+; not clicked on a real Mac yet. If it opens the wrong page, switch to `Privacy_ScreenCapture`.
- When both permissions are missing the button opens the microphone pane first; after the member grants it and tries again, the system audio error shows its own button.

## Desktop auto-update spec
- The spec now reverses "no auto-update": Tauri 2 updater, `latest.json` on GitHub Releases (public repo, no token), private signing key only on the maintainer's Mac, never in the repository or CI (there is no macOS CI). The prompt never appears during a recording. Docs only; nothing is built yet.
- Assumptions, none verified: no quarantine attribute on an update the app downloads itself; the "Recapz Local" certificate keeps the microphone and system-audio grants across updates; the updater can overwrite `/Applications/Recapz.app` for a non-admin member; one `darwin-aarch64` entry is enough while the build is not universal. The end-to-end test in the last auto-update PR checks each one.
- Open: the endpoint and key handling assume the repository stays public. A private repository needs the endpoint moved to the Recapz server.

## Desktop update release step
- `make desktop-dmg VERSION=x.y.z` signs the update with the key at `~/.tauri/recapz-updater.key` (override with `UPDATER_KEY`), passed as `TAURI_SIGNING_PRIVATE_KEY_PATH`; the password comes from `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. It writes `latest.json` with one `darwin-<arch>` entry for the build Mac (`desktop-latest.sh`, covered by `scripts/test-desktop-latest.sh`).
- The pre-commit hook refuses a staged `*.key` file or a minisign secret key body, and `*.key` is git-ignored. A test builds a scratch repo for each case.
- Not verified: that the Tauri CLI honours `TAURI_SIGNING_PRIVATE_KEY_PATH` (if not, switch to `TAURI_SIGNING_PRIVATE_KEY` with the file content), that `--config` with only `version` merges into `tauri.conf.json`, that the `.app.tar.gz` and `.sig` land in `bundle/macos/` when `--bundles app,dmg`, and a real build. Nothing here was built (no key use, no release made).

## Desktop updater key variable
- The Tauri CLI did not honour `TAURI_SIGNING_PRIVATE_KEY_PATH` ("A public key has been found, but no private key", seen on the maintainer's Mac). `make desktop-dmg` and `desktop-update-test.sh` now set `TAURI_SIGNING_PRIVATE_KEY` to the content of the key file for the build command only. The content is never echoed: make prints the recipe unexpanded, and it is not on a command line.
- Not verified: a full build with the new variable; the other two PR 3 doubts (the `--config` merge and the `bundle/macos/` file names) are still open.
