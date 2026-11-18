# Hosted speech and language providers

Status: **decided (2026-11-18), not implemented.** Supersedes the earlier plan where local whisper was the default
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
| Deploy | 2 vCPU, 4 GB RAM, 40 GB disk, `max_concurrent_calls = 2`. Chromium per call is the memory driver and WAV audio (about 115 MB per hour) the disk driver |

## Why Deepgram

It is the only option whose docs cover Vietnamese, speaker labels and timestamps in one pre-recorded call, with a
streaming API on the same models. OpenAI's diarization model is being retired and its transcription returns no
timestamps; AssemblyAI is cheaper but needs upload-and-poll code and has no confirmed speaker labels for `vi`.
No provider has been measured on Vietnamese; the check is a consented sample before launch.

## Cost per meeting (list prices, to verify)

Batch about $0.31 per hour of audio, streaming about $0.46 per hour (*unverified*), plus a few cents of LLM. A one
hour meeting with the live view on costs about $1. The monthly cap in config is the guard.

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
| 4 | `feat(core): store speaker labels from segments` | Rows match segments; retried job does not duplicate; full workspace tests |
| 5 | `feat(speech): add the Deepgram batch transcriber` | Mock-server tests for parsing, speaker mapping, retry on 429, no retry on 401, no key or text in errors |
| 6 | `feat(server): add speech provider config and disclosure` | Missing key fails at start; `/api/me` reports the provider |
| 7 | `feat(web): name the speech provider on upload and privacy` | jsdom test; matches the prototype per `docs/ui-parity.md` |
| 8 | `feat(core): record usage and enforce the monthly cap` | Usage rows per meeting; cap pauses new jobs; admin sees estimated cost |
| 9 | `feat(speech): add the Deepgram streaming transcriber` | Mock WebSocket test; failure never stops recording (Live Ask ticket #191 uses it) |
| 10 | `refactor(speech): remove whisper and CUDA` | Build, Makefile, config, `docs/running-locally.md` and README no longer mention it |
| 11 | Manual check, no PR | A 2 hour file and a live call on a real key confirm limits, Vietnamese quality and the 6 s Live Ask bar |

## Open questions

- Exact Deepgram and OpenAI settings that give no training and no retention (*unverified*).
- Whether Deepgram streaming accepts the notetaker's 16 kHz mono i16 frames as they are (expected yes).
