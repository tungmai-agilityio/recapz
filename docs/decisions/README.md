# Decision records

One file per decision: what was decided, why, and what is still unverified. Specs (`docs/specs/`) say what to build;
these say why a choice was made.

| Record | Date | Status |
|---|---|---|
| `hosted-speech.md` | 2026-11-18 | Hosted speech (Deepgram) and provider-agnostic LLM (OpenAI default); local whisper dropped |
| `provider-naming-removed.md` | 2026-11-20 | Providers not named to members |
| `local-code-signing.md` | 2026-11-23 | Self-signed local certificate for the macOS app |
| `desktop-auto-update.md` | 2026-11-25 | Updater with `latest.json` on GitHub releases, key only on the maintainer's Mac |
| `meeting-detection-by-microphone.md` | 2026-11-25 | Detect calls by microphone holder only, always ask |
| `zoom-join-path.md` | 2026-11-27 | Zoom join path: Meeting SDK bot in a sidecar; plan in `docs/specs/zoom-notetaker.md` |
| `speech-model-comparison.md` | superseded | Local model plan; kept for the error-rate method |
