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

## Open issues
- Google sign-in has not been exercised against real Google credentials.
- `BANNED_NAMES` is not set anywhere, so `scripts/banned-names.sh` skips. The list must be provided outside the repo.
- Dependabot opened major bumps (#12 tower-http 0.7, #13 toml 1.x, #14 @types/node 26). Not reviewed or merged.
- No CSRF token on state-changing routes yet (`POST /auth/logout`, `POST /api/uploads`); only `SameSite=Lax` cookies protect them. Add before any public exposure.
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

## Blockers
- `gh pr merge` is denied by the session's permission system even with an allow rule, so PRs are opened as a stack and merged by the maintainer. Week 2 hit the same block ("Merge Without Review") on #33.
- Week 2 merge order: #33 → #34 → #35 → #36 → #37 → #38 → #39 (stacked, each against the previous branch; after each squash-merge retarget the next to `main`). Independent: #40 (eval), #28 and #29 (docs), this notes PR.
- Week 3-5 LLM stack: #42 → #43 → #44 → #45 (each against the previous branch); retarget the next to `main` after each squash-merge.
- Week 3-5 merge order. LLM stack: #42 → #43 → #44 → #45. Week 2 stack: #33 → … → #39, then #48 (report job). Then `chore/integration-wk3` (merges #48, #45 and #47; it is a base branch, not a PR): its PRs are #49 (LLM report model, `[llm]` config) → #50 (queue the report job) → #51 (failed jobs mark the meeting failed) → #52 (report endpoint) → #53 (Report page sections), and independently #54 (notetaker session rules) and #55 (purge). #47 (opt-out and speaker matching) merges to `main` on its own. After the stacks reach `main`, retarget #49, #54 and #55 to `main` (they are based on the integration branch); #50-#53 follow #49 in order.
- The Chromium notetaker driver is not built: join flow, platform selectors, audio capture script and the join notice in call chat. It needs a real Meet call and the notetaker's Google account to verify, so it is a live-call blocker, not a code gap I could close here. The pieces that need no browser (session rules, WAV writer, opt-out, speaker matching, purge) are built and tested.

## Questions for review
- Confirm the Zoom Meeting SDK bot for v1 (#29) and who owns the Zoom developer account.
- Which speech model and Zoom join path to choose depends on real recordings (needs team recordings with consent).
