# Live Ask plan

Status: **design agreed with the maintainer (2026-11-17); nothing is built.** Starts after the Meet live
check passes (`docs/specs/meet-notetaker.md` row 16), before Zoom. Meet only until the Zoom notetaker exists.

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

`CLAUDE.md` and `docs/spec.md` change from "only transcript text goes to LLM providers" to "only transcript
text, and text of documents the uploader marked 'May be sent to the language model', go to LLM and
embedding providers". This PR makes that change.

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
between shorter windows, a smaller model or hosted speech (`docs/decisions/hosted-speech.md`). No later rows start
before that.

## PR breakdown

Each row is one concern of about 300 source lines or fewer, in dependency order.

| # | PR | Depends on | Done when |
|---|---|---|---|
| 0 | `docs: plan Live Ask` (this) | — | Approved by the maintainer |
| 1 | `fix(build): build whisper with CUDA for release` | — | `make release` enables `whisper` and the CUDA feature; a GPU run is logged |
| 2 | Spike: tap live audio and run windowed whisper | 1 | Latency and Vietnamese/English WER per window size written to `docs/live-ask-spike.md`; no product code merged |
| 3 | `feat(llm): add hosted embeddings` | 0 | Mock HTTP tests; `[llm]` config; meetings re-index on the new embedder id |
| 4 | `feat(core): store and ingest member documents` | 3 | Migration, upload with limits, per-document toggle, hard delete of file, chunks and vectors. Full workspace tests |
| 5 | `feat(core): search documents with meetings` | 4 | Uploader-only rule; toggle-off documents keyword only; a test that another member's documents never appear |
| 6 | `feat(server): stream the live transcript` | 2 | In-memory windows, `GET /api/meetings/{id}/live` SSE, GPU priority, opt-out clears it, notice text |
| 7 | `feat(llm): add the live answer prompt` | 5 | `live_answer.v1` with schema; 30-question eval set; `cargo run -p eval` meets the bar |
| 8 | `feat(server): answer live questions` | 6, 7 | `POST /api/meetings/{id}/live-ask`; access, rate limit and toggle tests |
| 9 | `feat(web): live view with Ask now` | 8 | Matches the prototype per `docs/ui-parity.md` S6, both skins, mobile |
| 10 | `feat(web): manage my documents` | 4 | Upload, toggle, delete; matches the prototype |

New dependencies, each with its reason in its PR: a PDF text extractor in row 4.

## Later

- Automatic question detection from other participants' speech.
- Linking documents to clients and calendar events; per-call filters.
- DOCX, web links, Drive.
