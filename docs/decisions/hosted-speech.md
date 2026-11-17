# Hosted speech provider decision

Status: **decided, not implemented.** Local whisper (`ggml-small` first) stays the default. This records the plan for an optional hosted speech adapter. Facts below were read from provider docs on 2026-11-12. Items marked *unverified* must be checked before building.

## Options

| | Deepgram Nova-3 | OpenAI | AssemblyAI Universal-2 |
|---|---|---|---|
| Vietnamese | `vi` listed, monolingual and multilingual ([models](https://developers.deepgram.com/docs/models-languages-overview)) | `gpt-transcribe` language list does not name `vi` (*unverified*) | Listed in the ">10–25% WER" group ([languages](https://www.assemblyai.com/docs/pre-recorded-audio/supported-languages)) |
| Speaker labels | Included for pre-recorded audio; `speaker` per word and utterance ([diarization](https://developers.deepgram.com/docs/diarization)) | Only `gpt-4o-transcribe-diarize`, deprecated, shuts down 2027-02-26 ([guide](https://developers.openai.com/api/docs/guides/speech-to-text)) | +$0.02/h; support for `vi` not confirmed |
| Timestamps | Per word | None documented for `gpt-transcribe` | Per word |
| Cost per hour | $0.31 multilingual, $0.26 monolingual ([pricing](https://deepgram.com/pricing)) | $0.27 `gpt-transcribe`, $0.36 diarize ([pricing](https://developers.openai.com/api/docs/pricing)) | $0.17 with diarization ([pricing](https://www.assemblyai.com/pricing)) |
| Limits | 2 GB per file; 504 after 10 min of processing ([pre-recorded](https://developers.deepgram.com/docs/pre-recorded-audio), page may be dated) | 25 MB per file, so chunking is required | Async upload, then polling |

## Decision

1. **First provider: Deepgram Nova-3 multilingual.** It is the only option whose docs cover Vietnamese, speaker labels and timestamps in one synchronous call without chunking.
2. **Not now: OpenAI** (its only diarization model is being retired, and click-to-seek needs timestamps) and **AssemblyAI** (cheaper, but upload-and-poll code and no confirmed speaker labels for `vi`). Revisit when the eval set exists.
3. **Privacy rule amendment.** "Audio never leaves the server" becomes "Audio never leaves the server unless the deployment opts in to a hosted speech provider." It is off by default, explicitly opted in per deployment and shown in the UI. The wording change in `docs/spec.md`, `CLAUDE.md` and `config.example.toml` lands in PR 1 below, not before.

No provider has been measured on Vietnamese. Accuracy waits on the consented test set described in `docs/decisions/speech-model-comparison.md`.

## Design

- **Trait.** `Transcriber::transcribe(&[f32], lang)` is unchanged. `Segment` gains `speaker: Option<String>`. Whisper returns `None`.
- **Adapter.** `HostedTranscriber` in `crates/speech`, behind a `hosted` feature. It encodes the 16 kHz mono samples as 16-bit WAV (about 115 MB per hour) and sends one blocking `reqwest` call. The runner already calls the transcriber inside `spawn_blocking`. Sending decoded WAV means every format the decoder reads works.
- **Speaker labels.** The transcribe job creates `speakers` rows (`Speaker 1`, `Speaker 2`, …) and sets `segments.speaker_id`. Uploads get speaker labels for the first time. Notetaker speaker matching can rename them later.
- **Config.**
  ```toml
  [speech]
  provider = "local"            # "local" or "deepgram"
  api_key = ""
  model = "nova-3"
  base_url = ""                 # empty uses the provider default; tests point it at a mock
  audio_leaves_server = false   # must be true for any hosted provider
  ```
  `speech_model` stays top-level for the local path. A hosted provider without `audio_leaves_server = true` makes the server refuse to start.
- **Disclosure.** `/api/me` reports the speech provider. When it is hosted, the upload screen and Recording & privacy show "Audio is sent to {provider} for transcription."
- **Errors and retries.** Inside one call, 429, 5xx and timeouts get two retries with backoff that honours `Retry-After`. 400, 401, 402 and 413 fail at once with the status and the provider's error code. Errors and logs never hold transcript text or the key. The job runner's retry handles the rest.
- **Long audio.** No chunking in v1. If a 2-hour file hits the 10-minute limit, split into 60-minute chunks. The risk is that speaker labels do not line up across chunks.
- **Out of scope.** Live view streaming, cost tracking per meeting, other providers.

## PRs, in order

| # | PR | Done when |
|---|---|---|
| 1 | `docs: amend the audio rule for hosted speech` | New wording in spec, CLAUDE.md and config comment is merged with maintainer approval |
| 2 | `feat(core): store speaker labels from segments` | Segments with speakers create matching `speakers` rows; a retried job does not duplicate them; `None` leaves `speaker_id` NULL. Full workspace tests |
| 3 | `feat(speech): add hosted transcriber` | Mock-server tests: parsing synthetic JSON, speaker mapping, retry then success on 429, no retry on 401; WAV round-trips through `decode_to_16k_mono`; no key or text in errors. About 300 lines |
| 4 | `feat(server): add speech provider config` | Startup guard has a test; `/api/me` reports the provider |
| 5 | `feat(web): disclose hosted speech on upload and privacy` | jsdom test shows the line only for a hosted provider; matches the prototype per `docs/ui-parity.md` |
| 6 | Manual check, no PR | A 2-hour file on a real key confirms the time limit and Vietnamese quality |

## Open questions

- Deepgram data retention and opt-out from model training (*unverified*; read the terms before any real meeting is sent).
- Whether the notetaker's join message in call chat should also name the provider (proposed: yes).
