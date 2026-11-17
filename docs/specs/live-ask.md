# Live Ask

Status: **design agreed with the maintainer (2026-11-17); nothing is built.** Starts after the Meet live
check passes (`docs/specs/meet-notetaker.md` row 16), before Zoom. Meet only until the Zoom notetaker exists.
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
| Live transcript | Whisper on the server GPU over short windows, a small model for live and `large-v3-turbo` after the call. Shown in the live view, held in memory only, dropped when the call ends. The stored transcript still comes from the post-call run |
| GPU priority | Live windows first; post-call transcribe jobs wait while any call is live |
| Knowledge base | Per member: their uploaded documents plus the meetings they can already see |
| Document access | Uploader only |
| Search scope | All of the member's documents and visible meetings; every citation names its document or meeting |
| Document types | Text-layer PDF, Markdown, TXT. 20 MB per file, 500 MB per member. No OCR |
| Leaving the server | A per-document toggle "May be sent to the language model", default off, covering both embeddings and answers. Documents with it off are found by keyword search only and shown as cited passages with no generated answer |
| Embeddings | Hosted, through an embeddings call on the existing OpenAI-compatible adapter (`POST /v1/embeddings`), configured under `[llm]`. Replaces `hash-256`; meetings re-index through `enqueue_stale` |
| API | `POST /api/meetings/{id}/live-ask` with its own prompt `live_answer.v1`. Reuses `search`, the Ask rate limit, the SSE event names and the citation shape, with citations that point at a document or a meeting |
| Consent | The join notice says the call is transcribed live. Chat opt-out stops live transcription at once, clears the in-memory transcript and ends running answers, on top of the existing hard delete |
| Audio | Never leaves the server |

## Rule change

`CLAUDE.md` and `docs/spec.md` already say that only transcript text, and text of documents the uploader marked
"May be sent to the language model", go to LLM and embedding providers.

## What exists today

- Call audio reaches the server while the call is live: 100 ms frames of 16 kHz mono i16 from
  `crates/notetaker/src/meet/capture.js` through `DriverEvent::Audio`. Only the WAV writer in
  `crates/server/src/notetaker.rs` reads them.
- Whisper runs once over the finished file (`crates/core/src/transcribe_job.rs`). No timings are recorded
  (`docs/decisions/speech-model-comparison.md`). The build has no GPU feature, and `make release` does not enable
  `whisper`.
- Ask: `POST /api/ask` streams `token`, `citations`, `done`. `crates/core/src/search.rs` merges FTS5 and
  vector results with reciprocal rank fusion, filtered by `m.owner_id = ? OR m.share = 'workspace'`.
- The only embedder is `HashEmbedder` (`hash-256`), lexical and not cross-language.
- There is no `GET /api/events` and no live view.

## Success bar

- 6 s or less from pressing Ask now to the answer card.
- At least 80% correct with the right citation, or a correct "not in your documents", on a 30-question
  eval set built from one real client's documents.

If the spike (row 2) misses the latency target, stop and write up the numbers. The maintainer then decides
between shorter windows, a smaller model or hosted speech (`docs/decisions/hosted-speech.md`). Only the live rows
(6 to 9) wait for the spike; the document rows (3 to 5 and 10) do not depend on live audio and may run in parallel.

## Acceptance

- [ ] Ask now returns a cited answer card in 6 s or less from the button press.
- [ ] At least 80% of a 30-question eval set built from one real client's documents is answered correctly with the
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
| 1 (#186) | `fix(build): build whisper with CUDA for release` | tooling | none | `Makefile`, `crates/server/Cargo.toml` | small | `make release` builds; a GPU run is logged; fmt and clippy |
| 2 (#187) | `docs(speech): measure windowed live whisper` | speech | 1 | `docs/live-ask-spike.md` | small | docs only; no product code merged. Latency and Vietnamese/English WER per window size |
| 3 (#188) | `feat(llm): add hosted embeddings` | llm | none | `crates/llm/` | medium | `-p llm` with mock HTTP tests; `[llm]` config; meetings re-index on the new embedder id |
| 4 (#189) | `feat(core): store and ingest member documents` | core | 3 | `crates/core/` | medium | Full workspace. Migration, upload limits, per-document toggle, hard delete of file, chunks and vectors |
| 5 (#190) | `feat(core): search documents with meetings` | core | 4 | `crates/core/src/search.rs` | medium | Full workspace. Uploader-only rule; toggle-off documents keyword only; another member's documents never appear |
| 6 (#191) | `feat(server): stream the live transcript` | server | 2 | `crates/server/` | medium | Full workspace. In-memory windows, `GET /api/meetings/{id}/live` SSE, GPU priority, opt-out clears it |
| 7 (#192) | `feat(llm): add the live answer prompt` | llm | 5 | `crates/llm/prompts/`, `crates/eval/` | medium | `cargo run -p eval` meets the bar; `live_answer.v1` with schema; 30-question eval set |
| 8 (#193) | `feat(server): answer live questions` | server | 6, 7 | `crates/server/` | medium | Full workspace. `POST /api/meetings/{id}/live-ask`; access, rate limit and toggle tests |
| 9 (#194) | `feat(web): live view with Ask now` | web | 8 | `web/src/` | medium | Web tests and lint; matches the prototype per `docs/ui-parity.md` S6, both skins, mobile |
| 10 (#195) | `feat(web): manage my documents` | web | 4 | `web/src/` | medium | Web tests and lint; upload, toggle, delete; matches the prototype |

New dependencies, each with its reason in its PR: a PDF text extractor in row 4.

## Open questions

- Meet live check (`docs/specs/meet-notetaker.md` row 16) must pass before any ticket starts. The maintainer runs it
  and reports the result.
- `CLAUDE.md` describes `make release` as a production build with CUDA, but the target runs a plain
  `cargo build --release` with no `whisper` feature. Row 1 settles which is right.

## Out of scope

- Automatic question detection from other participants' speech.
- Linking documents to clients and calendar events; per-call filters.
- DOCX, web links, Drive.
