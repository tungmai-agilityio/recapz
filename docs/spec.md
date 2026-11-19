# Recapz spec

Recapz is a private meeting notetaker for one team on one server. A notetaker joins Google Meet and Zoom,
transcribes mixed Vietnamese and English with a hosted speech provider, writes a meeting report, and lets the team
search every meeting with cited answers. Everything ships as one Rust binary.

## Vocabulary

| Term | Meaning |
|---|---|
| Notetaker | Bot that joins a call, announces itself in chat and records audio |
| Meeting report | Summary, decisions, open questions, chapters, action items, talk time |
| Transcript | Timestamped, speaker-labelled text; clicking a line seeks the audio |
| Speaker ribbon | Strip showing who spoke when; used as scrubber and in lists |
| Ask | Question box over all accessible meetings, answers cite exact moments |
| Live view | Running transcript and talk time during a call |
| Live Ask | During a call, a cited answer from your documents and past meetings, private to you (`docs/specs/live-ask.md`) |
| Browser recording | A member records a call from their own browser: the meeting tab's audio plus their microphone ("Meeting tab"), or the microphone only ("In-room"), streamed live to the server (`docs/specs/browser-recording.md`) |
| Recorder bar | In the live view of the tab that is recording: elapsed time, input levels, Stop, Stop and discard |
| Upload | Process an audio or video file the notetaker did not record |
| Members | Google sign-in; admins invite by email, remove people, change roles |
| Recording & privacy | Join rules, notetaker name/message, retention, purge one participant |

Out of scope for v1: catch-up, talk stats, inbox digest, follow-ups, connections beyond calendar.

Opt-out is required: anyone typing "opt out" in call chat makes the notetaker leave and the recording,
transcript and report for that meeting are deleted.

Browser recording has no bot in the call, so no chat notice and no "opt out" phrase. Before every browser
recording the member ticks "I have told everyone in this meeting that it is recorded and sent to <speech
provider>"; recording cannot start until it is ticked, and the dialog offers a notice naming the speech provider
to copy into the call chat. The recorder can stop and discard at any time, which hard-deletes the meeting, its
audio and any live text, as the chat opt-out does.

## Architecture

One process: axum JSON API + SSE, job runner, speech, embeddings, LLM adapters. On disk: `control.sqlite`,
one `recapz.sqlite` per tenant under `data/tenants/<tenant_id>/`, and an `audio/` folder. WAL mode,
one writer, many readers. Times are UTC milliseconds; IDs are UUIDv7 text.

## Meeting pipeline

`scheduled → joining → recording → transcribing → labelling → reporting → indexing → ready`
(terminal: `failed`, `opted_out`). Each arrow is a row in the `jobs` table so a restart resumes work.
Uploads start at `uploaded` and enter the pipeline at `transcribing`.

## API (v1)

Cookie session; tenant comes from the session. Every handler checks the member can see the meeting.

- `GET /auth/google`, `GET /auth/callback`, `POST /auth/logout` — Google sign-in, invite-only
- `GET /api/me`, `GET /api/meetings`, `POST /api/uploads`, `GET /api/health`
- Later: meeting detail/transcript/ribbon/audio, `POST /api/ask`, `GET /api/events`, members, invites,
  settings, participant purge.

## Language models

Two adapters (Anthropic Messages, OpenAI-compatible Chat Completions) behind a `Provider` trait and a
per-task `Router`. Reports are written once and stored. Ask sends only top retrieved chunks. Outputs are
JSON validated against a schema. Only transcript text, the text of a question a member asks, and text of documents the uploader marked "May be sent to the language model", go to the configured LLM and embedding providers. See "Data sent to providers" below.

## Web

React + Vite + TypeScript + Tailwind. Daylight (default) and Ink skins, each light and dark. Be Vietnam Pro
for text, JetBrains Mono for times. Amber `signal` is only for the playhead, recording state and focus
rings; speakers own all other colour.

The prototype is the visual spec. Match it first: for each screen, start from the prototype state (layout,
components, mobile layout under 900 px, both skins), build or reuse the shared primitives in `web/src/ui/`,
then wire real data. A screen is not done while its rows in `docs/ui-parity.md` are unticked. Anything the
backend cannot serve yet is hidden, never faked.

## Security

Invite-only Google sign-in, no passwords. Cookies `HttpOnly`, `SameSite=Lax` (`Secure` behind HTTPS).
Permission checks in every query. Secrets never logged. Logs hold IDs, not transcript text.
Deletes are hard deletes, including search rows and audio files. A meeting can be deleted by its owner or
by an admin, and not while it is joining, recording or processing.

## Data sent to providers

- Audio goes only to the configured speech provider (Deepgram first): the live stream during a call and the
  recorded file after it. Nothing else is sent to it. A browser recording streams the member's microphone and
  shared tab audio to the Recapz server only; the server sends it on to the speech provider as for any call.
- Transcript text, the text of a question a member asks, and text of documents the uploader marked "May be sent
  to the language model" go only to the configured LLM and embedding providers (OpenAI by default). Documents with
  the toggle off (the default) are never sent.
- Provider accounts must use no-training and no-retention settings. Deployment-wide keys live in the environment
  file, never in `config.toml`, logs or the database.
- The Recording & privacy screen, the notetaker's join message in call chat, and the browser recording consent
  box and copied notice name the speech provider.
- Deletes are hard deletes on our side. Providers keep nothing beyond transient processing under those settings,
  so Recapz makes no provider-side delete calls.
- If a provider is unavailable, audio stays on disk and jobs retry; there is no automatic second provider.
