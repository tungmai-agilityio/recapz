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
- Notetaker logic is browser-free: `Session` (lobby 10 min, alone 2 min, call ended, opt-out from anyone but the notetaker itself) and `WavWriter`. A driver must feed it participant samples about every 250 ms and send `from_self` correctly on chat events.
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
- Tested against a mock S3 server on localhost (put, multipart put of a 20 MiB file, head, ranged get, delete), which does not check signatures. Not verified against Hetzner Object Storage: request signing, path-style addressing, its multipart behaviour, and `HEAD` content length. Not built yet, left for the next PR: audio retention (default 90 days, admin-settable) and the privacy-screen line naming object storage.

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
