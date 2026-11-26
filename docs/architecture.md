# Architecture overview

A map of where things live, checked against the repository tree on 2026-11-25. For behaviour and rules see
`docs/spec.md`; for why, `docs/decisions/`. Anything not checked is marked *unverified*.

## Pieces

| Piece | Where | Role |
|---|---|---|
| Server binary | `crates/server` | axum HTTP routes (`routes.rs`, `routes/`), config, job runner (`runner.rs`), live transcript (`live.rs`), notetaker loop (`notetaker*.rs`), browser-recording socket (`browser_call.rs`), calendar sync, embedded web app (`web.rs`) |
| Core | `crates/core` | Domain, control and per-tenant SQLite, additive migrations (`migrations/`), job queue (`jobs.rs`), pipeline jobs (`transcribe_job`, `label_job`, `report_job`, `index_job`, `archive_job`), search, documents, usage and cap, retention, hard-delete (`purge.rs`) |
| Speech | `crates/speech` | `Transcriber` trait, Deepgram batch (`deepgram.rs`) and streaming (`stream.rs`), upload decoding (`decode.rs`) |
| LLM | `crates/llm` | `Provider` trait, OpenAI-compatible and Anthropic adapters, report and Ask prompts (`prompts/`, versioned) |
| Notetaker | `crates/notetaker` | Browser driver for Google Meet, session rules, opt-out detection, WAV writer; no browser needed for its unit tests |
| Eval | `crates/eval` | `cargo run -p eval`: speech word error rate, Ask, live-answer and report-prompt scoring (`eval report`). `eval ask <set.json> [--embedder hash\|hosted]` prints the pass rate at cosine cut-offs 0.0 to 0.6; each embedder owns its cut-off (`Embedder::min_similarity`, hash 0.2, hosted 0.2 until measured) |
| Web | `web/` | React + Vite app: `src/pages`, shared primitives in `src/ui`, recording capture in `src/capture` |
| Desktop | `desktop/` | Tauri 2 macOS app in its own Cargo workspace (CI never builds it): shell and sign-in (`main.rs`), Core Audio capture (`audio/`), microphone-holder detection (`detect.rs`), prompt window (`prompt.rs`, `ui/prompt.html`), tray dot (`dot.rs`), updater (`update.rs`) |
| Deploy | `deploy/`, `scripts/` | systemd units, Caddyfile, backup timer, provisioning and release scripts, git hooks |

## Data flow

1. **Audio in.** Four routes lead to the same pipeline: the Meet notetaker (Chromium on the server), an upload, a
   browser recording (`POST /api/recordings` plus the `/api/meetings/{id}/record` socket), and the macOS app, which
   sends the same frames as a browser recording.
2. **Live.** Frames also go to the streaming transcriber; text is held in memory for the live view and Live Ask and
   dropped when the call ends.
3. **After the call.** Jobs run in order: transcribe (batch, speaker labels), label, report (LLM), index (embeddings),
   then `ready`. Each step is a `jobs` row so a restart resumes.
4. **Ask.** Search merges keyword and vector results over the meetings and documents the member may see, then the LLM
   answers with citations.

## Storage

`control.sqlite` plus one `recapz.sqlite` per tenant under `data_dir`, and an `audio/` folder; recordings stay on server
disk (object storage is built but not enabled). Deletes are hard deletes. Secrets are in the environment file only.

## Not covered here

Request-level sequence for each route, the Google sign-in flow, and the exact migration list (*unverified*: not
re-read for this page). See `crates/server/src/routes.rs` and `crates/core/migrations/`.
