# Hosted speech and language providers

Status: **decided (2026-11-18), implemented (PRs 4 to 10 merged).** Real-key checks still to run are listed under "Not yet run". Supersedes the earlier plan where local whisper was the default
and a hosted provider was an opt-in. Provider facts were read from provider docs on 2026-11-12; items marked
*unverified* must be checked before a real meeting is sent.

## Decisions

| Area | Decision |
|---|---|
| Speech provider | Deepgram Nova-3 multilingual, batch and streaming, behind the `Transcriber` trait |
| Modes | Batch on the recorded file after the call gives the stored transcript and speaker labels. A streaming WebSocket feeds the live view and Live Ask |
| Speaker labels | Deepgram diarization on the batch pass. The live view shows text without speakers |
| LLM and embeddings | Provider-agnostic adapters, OpenAI as the default, per-task `Router` kept so reports and live answers can use different models. Anthropic and any OpenAI-compatible server stay as adapters |
| Privacy rule | Audio goes only to the configured speech provider; transcript, question and marked-document text only to the configured LLM and embedding providers; providers are named to members. See `docs/spec.md`, "Data sent to providers" |
| Documents | The per-document "May be sent to the language model" toggle stays, default off |
| Retention | Accounts run with no-training and no-retention settings (*unverified*: exact Deepgram and OpenAI settings). Deletes stay local hard deletes. No provider-side delete calls |
| Keys | One key per provider for the deployment, in `/etc/recapz/recapz.env`, never `config.toml`, logs or the database |
| Cost | Per-meeting usage (audio seconds, tokens) stored; estimated cost shown to admins; a monthly cap in config pauses new jobs |
| Failure | Audio stays on disk, jobs retry with backoff, streaming failure never blocks recording and the live view says "live transcript unavailable". Auth and quota errors fail fast with an admin alert. No automatic second provider |
| Local whisper | Dropped: `whisper-rs`, the CUDA build and model files leave the build and docs. The `Transcriber` trait stays so a local engine can return. Upload decoding (symphonia) stays |
| Recording storage | Object storage provider is open (undecided); recordings are on server disk for now (no bucket enabled). The 2026-11-18 Hetzner choice is superseded: hosting moved to DigitalOcean. With a private S3-compatible bucket configured, the notetaker still writes WAV to local disk during the call; after a successful batch transcript the file is uploaded to the bucket and the local copy removed. Playback streams from the bucket with range requests. Deleting a meeting, opt-out or participant purge hard-deletes the objects. The bucket is private, one deployment key in the env file, and is named on the privacy screen as a place audio is kept |
| Deploy | 2 vCPU, 4 GB RAM, 40 GB disk or more (the last Droplet had 79 GB; it has since been deleted, see `docs/deploy.md`), `max_concurrent_calls = 2`. Chromium per call is the memory driver and in-call WAV (about 115 MB per hour, local only until uploaded) the disk driver; recordings stay on server disk until a storage provider is chosen |

## Why Deepgram

It is the only option whose docs cover Vietnamese, speaker labels and timestamps in one pre-recorded call, with a
streaming API on the same models. OpenAI's diarization model is being retired and its transcription returns no
timestamps; AssemblyAI is cheaper but needs upload-and-poll code and has no confirmed speaker labels for `vi`.
No provider has been measured on Vietnamese; the check is a consented sample before launch.

## Cost per meeting (list prices, to verify)

Batch about $0.31 per hour of audio before speaker labels; with diarization (about $0.0020 per minute, a paid add-on) and
Nova-3 multilingual batch at about $0.0058 per minute (promo price) it is about $0.47 per hour (*second-hand from
Deepgram's pricing pages on 2026-11-16, unverified*). Streaming about $0.46 per hour (*unverified*), plus a few cents of
LLM. A one hour meeting with the live view on costs about $1. The monthly cap in config is the guard.

## Design

- `Segment` gains `speaker: Option<String>`; the transcribe job creates `speakers` rows (`Speaker 1`, `Speaker 2`, ...)
  and sets `segments.speaker_id`. A retried job must not duplicate them.
- `crates/speech` holds `DeepgramTranscriber` (batch, one blocking call on 16-bit WAV, 429, 5xx and timeouts retried
  twice with `Retry-After`) and `DeepgramStream` (WebSocket, 100 ms frames already produced by the notetaker).
- Errors and logs never hold transcript text or the key.
- Long audio: no chunking in v1. If a 2 hour file hits the provider's processing limit, split into 60 minute chunks;
  speaker labels may not line up across chunks.
- `/api/me` reports the speech provider so the upload screen and Recording & privacy can name it.

## PRs, in order

| # | PR | Done when |
|---|---|---|
| 1 | `docs(spec): replace the audio rule with provider disclosure` | Merged with maintainer approval |
| 2 | `docs(decisions): record the hosted provider decision` (this file) | Merged |
| 3 | `docs(live-ask): use the streaming transcript` and `docs(deploy): size a CPU-only host` | Merged |
| 4 | #227 `feat(core): store speaker labels from segments` | Rows match segments; retried job does not duplicate; full workspace tests |
| 5 | #237 `feat(speech): add the Deepgram batch transcriber` | Mock-server tests for parsing, speaker mapping, retry on 429, no retry on 401, no key or text in errors |
| 6 | #238 `feat(server): add speech provider config and disclosure` | Missing key fails at start; `/api/me` reports the provider |
| 7 | #241 `feat(web): name the speech provider on upload and privacy` | jsdom test; matches the prototype per `docs/ui-parity.md` |
| 8a | #243 `feat(core): keep recordings in object storage` | Upload after transcript, ranged playback, hard delete of objects on delete, opt-out and purge; mock S3 tests; one new dependency with its reason |
| 8 | #248 `feat(core): record usage and enforce the monthly cap` | Usage rows per meeting; cap pauses new jobs; admin sees estimated cost |
| 9 | #228 `feat(speech): add the Deepgram streaming client` | Mock WebSocket test; failure never stops recording (Live Ask ticket #191 uses it) |
| 10 | #251 `refactor(speech): remove whisper and CUDA` | Build, Makefile, config, `docs/running-locally.md` and README no longer mention it |
| follow-up | #250 `feat(core): choose a speech language per meeting` (issue #245) | Merged |
| follow-up | #261 `fix(speech): build the Deepgram client off the async thread` | Merged |
| follow-up | #268 `fix(server): install the TLS crypto provider at start` | Merged; live streaming works on main |
| 11 | Manual check, no PR | A 2 hour file and a live call on a real key confirm limits, Vietnamese quality and the 6 s Live Ask bar |

## Open questions

- Which S3-compatible provider holds recordings, and whether its region is near the San Francisco Droplet (*unverified*; if not, playback latency and cross-region transfer need a check). Audio retention period (suggested 90 days, admin-settable).

- Exact Deepgram and OpenAI settings that give no training and no retention (*unverified*).
- Whether Deepgram streaming accepts the notetaker's 16 kHz mono i16 frames as they are (expected yes).

## Real-key check (PR 11, maintainer-run)

A first 4-sample run was done on 2026-11-19 (see Results). The rows under "Not yet run" are still unverified against Deepgram and the object store. Run them with a real key, write the results under "Results" and tick the ticket (#226). Use only consented recordings; commit none.

Setup: `RECAPZ_SPEECH_KEY` (and the storage variables) in the environment, never in `config.toml`. Read the usage card on Recording & privacy before and after to compare the estimate with the provider's invoice.

| Check | How | Pass when |
|---|---|---|
| Vietnamese quality | 5 to 10 consented recordings, language `vi` | Written note per file: word errors seen, proper nouns, numbers |
| Mixed language | 3 or more recordings that switch Vietnamese and English, language `multi` | Compared with the same file in `vi`; note which reads better |
| `multi` with `diarize_model=latest` | Same mixed recordings | The request is accepted and speakers are labelled; if the provider rejects the pair, record the error text |
| Vietnamese speaker labels | A recording with 2 to 4 known speakers | Speaker count and turn changes match what was said |
| Long file | A 2 hour file, uploaded once | Finishes without a 504, or the provider limit is recorded and the split-file ticket is opened |
| Cost | The same month's invoice | Estimate in the usage card is within about 15% of the invoice |
| No retention | Dashboard or account settings | `mip_opt_out` is on for batch and streaming requests |
| Live latency | A live call with the Live Ask button, 10 tries | 6 s or less from the button to the answer in at least 9 of 10 |
| Storage | One upload, play it back, delete the meeting | Playback seeks; the object is gone afterwards |

### Results

First run, 2026-11-19, on four samples: public read-aloud and meeting-corpus audio, **not** consented team recordings. Real key, batch pass.

- Vietnamese word errors about 4.9 percent with language `vi`.
- `multi` about 7.6 percent on the same Vietnamese file, so `vi` reads better for Vietnamese-only audio.
- `multi` with `diarize_model=latest` was accepted (HTTP 200).
- English file with 4 speakers: 3 labels returned.
- Vietnamese file with 2 voices: 1 label returned.
- Usage card showed 53,950 micro-USD for about 7 minutes of audio.

### Not yet run

- [ ] Live latency, 10 tries with the Live Ask button
- [ ] 2 hour file
- [ ] Object storage playback (seek) and delete
- [ ] `mip_opt_out` seen in the provider dashboard
- [ ] Cost against the invoice
- [ ] 5 to 10 consented Vietnamese recordings
- [ ] Mixed-language recordings (3 or more, `multi` against `vi`)
