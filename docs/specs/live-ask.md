# Live Ask

Status: **design agreed with the maintainer (2026-11-17), speech changed to hosted streaming (2026-11-18); built and merged (#206 to #208, #254 to #257, #259, #265 to #266).** Ticket 9 (#192) is still open on GitHub although the prompt merged in #208. The 6 s bar, the 80% eval and the real-call checks are *unverified*: no real key or live Meet call has run them. Meet only until the Zoom notetaker exists.
Intent: #184

## What it is

During a call, a member presses **Ask now** in the live view. Recapz takes the question from the live
transcript, searches that member's documents and the past meetings they can see, and shows a short cited
answer they can say back to the client. Answers are private to the member who asked.

## Decisions

| Area | Decision |
|---|---|
| Who sees answers | Any member with access to the meeting can open its live view; each sees only their own asks |
| Trigger | Manual only: an Ask now button and hotkey. Automatic question detection is phase 2 |
| Question | The last 60 s of live transcript, or a clicked transcript line with its neighbours, plus optional typed text |
| Answer language | The language the question was asked in (Vietnamese or English), whatever the source language |
| Live transcript | Deepgram streaming over the call's 100 ms audio frames (`docs/decisions/hosted-speech.md`). Shown in the live view without speaker labels, held in memory only, dropped when the call ends. The stored transcript and speakers still come from the post-call batch run. If streaming fails the live view says "live transcript unavailable" and recording continues |
| Knowledge base | Per member: their uploaded documents plus the meetings they can already see |
| Document access | Uploader only |
| Search scope | All of the member's documents and visible meetings; every citation names its document or meeting |
| Document types | Markdown and TXT first; text-layer PDF later, after a text-extraction crate is vetted. 20 MB per file, 500 MB per member. No OCR |
| Leaving the server | A per-document toggle "May be sent to the language model", default off, covering both embeddings and answers. Documents with it off are found by keyword search only and shown as cited passages with no generated answer |
| Embeddings | Hosted, through an embeddings call on the existing OpenAI-compatible adapter (`POST /v1/embeddings`), configured under `[llm]`. Replaces `hash-256`; meetings re-index through `enqueue_stale` |
| API | `POST /api/meetings/{id}/live-ask` with its own prompt `live_answer.v1`. Reuses `search`, the Ask rate limit, the SSE event names and the citation shape, with citations that point at a document or a meeting |
| Consent | The join notice says the call is transcribed live. Chat opt-out stops live transcription at once, clears the in-memory transcript and ends running answers, on top of the existing hard delete |
| Audio | Goes only to the configured speech provider. The join notice no longer names it (rule dropped 2026-11-20, #323 to #326) |

## Rule change

The per-document toggle stays: only transcript text, questions and text of documents the uploader marked "May be sent to the language model" go to the LLM and embedding providers. The earlier rule that providers are named to members was dropped on 2026-11-20 (#323 to #326), and `docs/spec.md` no longer has a "Data sent to providers" section.

## What existed when this was written (history)

- Call audio reaches the server while the call is live: 100 ms frames of 16 kHz mono i16 from
  `crates/notetaker/src/meet/capture.js` through `DriverEvent::Audio`. Only the WAV writer in
  `crates/server/src/notetaker.rs` reads them.
- Transcription runs once over the finished file (`crates/core/src/transcribe_job.rs`); the hosted provider does it
  (`docs/decisions/hosted-speech.md`).
- Ask: `POST /api/ask` streams `token`, `citations`, `done`. `crates/core/src/search.rs` merges FTS5 and
  vector results with reciprocal rank fusion, filtered by `m.owner_id = ? OR m.share = 'workspace'`.
- The only embedder is `HashEmbedder` (`hash-256`), lexical and not cross-language.
- There is no `GET /api/events` and no live view.

## Success bar

- 6 s or less from pressing Ask now to the answer card.
- At least 80% correct with the right citation, or a correct "not in your documents", on a 30-question
  eval set built from synthetic client documents.

The 6 s bar is checked on a real streaming call in the live-transcript ticket (row 8). If it is missed, write up the numbers and the maintainer decides. The embedder and document rows (3 to 7 and 12) do not depend on live audio and may run in parallel.

## Acceptance

- [ ] Ask now returns a cited answer card in 6 s or less from the button press.
- [ ] At least 80% of a 30-question eval set built from synthetic client documents is answered correctly with the
      right citation, or with a correct "not in your documents".
- [ ] A member never sees another member's documents or asks, in search, citations or answers.
- [ ] Documents with "May be sent to the language model" off are found by keyword search only and never sent to a
      provider.
- [ ] A chat opt-out stops live transcription at once, clears the in-memory transcript and ends running answers.

## Tickets

One ticket is one PR (about 300 source lines small, 600 medium), in dependency order. Paths in `Touches` are
best guesses and are checked by the ticket's author. Testing scope follows the merge rule in `CLAUDE.md`.

| # | Title (`type(scope): summary`) | Area | Blocked by | Touches | Size | Testing scope |
|---|---|---|---|---|---|---|
| 1 (new) | `feat(speech): add the Deepgram streaming client` | speech | none | `crates/speech/src/`, `Cargo.toml` | medium | `cargo test -p recapz-speech`; mock WebSocket test |
| 3 (#188) | `refactor(core): make the embedder fallible` | core | none | `crates/core/src/embed.rs`, `index_job.rs`, `search.rs`, `crates/server/src/embedder.rs` | small | Full workspace |
| 4 (#197) | `feat(server): add the hosted embedder` | server, llm | 3 | `crates/server/src/embedder.rs`, `config.rs`, `runner.rs`, `Cargo.toml`, `config.example.toml` | medium | Full workspace; mock HTTP tests |
| 5 (#189) | `feat(core): store and ingest member documents` | core | 4 | `crates/core/migrations/tenant_006.sql`, `db.rs`, `jobs.rs`, `document.rs`, `purge.rs` | medium | Full workspace |
| 6 (#198) | `feat(server): add document routes and the ingest job` | server | 5 | `crates/server/src/routes.rs`, `routes/documents.rs`, `runner.rs`, `tests/` | medium | Full workspace |
| 7 (#190) | `feat(core): search documents with meetings` | core, server | 5 | `crates/core/src/search.rs`, `crates/server/src/routes/ask.rs`, `tests/ask.rs`, `crates/eval/src/ask.rs`, `web/src/api.ts` | medium | Full workspace; web tests for the `Citation` type |
| 8 (#191) | `feat(server): stream the live transcript` | server, core, speech | 1 | `crates/server/src/notetaker.rs`, `notetaker_loop.rs`, `runner.rs`, `routes/live.rs`, `crates/speech/src/transcribe.rs`, `crates/core/src/jobs.rs`, `transcribe_job.rs` | medium | Full workspace |
| 9 (#192) | `feat(llm): add the live answer prompt` | llm, server | 7 | `crates/llm/prompts/`, `crates/llm/src/ask.rs`, `crates/eval/` (synthetic fixtures) | medium | `cargo run -p eval`, `-p llm`, `-p eval` |
| 10 (#193) | `feat(server): answer live questions` | server, llm, core | 8, 9 | `crates/server/src/routes/live_ask.rs`, `routes.rs`, `tests/live_ask.rs`, `crates/llm/src/lib.rs` | medium | Full workspace |
| 11 (#194) | `feat(web): live view with Ask now` | web | 10 | `web/src/App.tsx`, `Shell.tsx`, `api.ts`, `pages/Live.tsx`, `docs/ui-parity.md` | medium | Web tests and lint; browser flow |
| 12 (#195) | `feat(web): manage my documents` | web | 6 | `web/src/pages/Documents.tsx`, `ui/Table.tsx`, `api.ts`, `App.tsx`, `Shell.tsx`, `docs/ui-parity.md` | medium | Web tests and lint |

New dependencies, each with its reason in its PR: none planned. PDF text extraction is a later ticket after a crate is vetted.

## Open questions

- Meet live check (`docs/specs/meet-notetaker.md` row 16) must pass before any ticket starts. The maintainer runs it
  and reports the result.
- None open. The old GPU build ticket (#186) and the local speed measurement (#187) are closed: no local GPU is used.

## Out of scope

- Automatic question detection from other participants' speech.
- Linking documents to clients and calendar events; per-call filters.
- DOCX, web links, Drive.
