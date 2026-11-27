# Recapz spec

Recapz is a private meeting notetaker for teams. One server can hold several workspaces (one per team), and a person can belong to more than one and switch from the account menu. A notetaker joins Google Meet (Zoom has no
join path yet; Zoom calls are recorded from the macOS app or uploaded), transcribes mixed Vietnamese and English with a hosted speech provider, writes a meeting report, and lets the team
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
| Browser recording | A member records a call from their own browser: the meeting tab's audio plus their microphone ("Meeting tab"), or the microphone only ("In-room"), streamed live to the server (`docs/specs/browser-recording.md`). In the macOS desktop app the same dialog records the Mac's system audio plus the microphone ("Meeting app") through native capture (`docs/specs/desktop-app.md`) |
| Recorder bar | In the live view of the tab that is recording: elapsed time, input levels, Stop, Stop and discard |
| Desktop app | macOS app (Tauri) that opens the web app and records the Mac's system audio plus microphone; asks "Record this meeting?" when a call app starts using the microphone (`docs/specs/desktop-app.md`, `docs/specs/meeting-detection.md`) |
| Upload | Process an audio or video file the notetaker did not record |
| Members | Google sign-in; admins invite by email, remove people, change roles |
| Recording & privacy | Join rules, notetaker name/message, retention, purge one participant |

Out of scope for v1: catch-up, talk stats, inbox digest, follow-ups, connections beyond calendar.

Opt-out is required: anyone typing "opt out" in call chat makes the notetaker leave and the recording,
transcript and report for that meeting are deleted.

Browser recording has no bot in the call, so no chat notice and no "opt out" phrase. Before every browser
recording the member ticks "I have told everyone in this meeting that it is recorded"; recording cannot start until it is ticked,
and the dialog offers a notice to copy into the call chat. The recorder can stop and discard at any time, which hard-deletes the meeting, its
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
- `GET /auth/desktop`, `POST /auth/desktop/exchange` — sign-in for the macOS app through the system browser
- `GET /api/me`, `GET /api/meetings`, `GET /api/meetings/upcoming`, `POST /api/meetings/join`, `POST /api/uploads`,
  `POST /api/recordings`, `GET /api/health`
- Per meeting: `/transcript`, `/report`, `/audio`, `/live`, `/record` (WebSocket), `/live-ask`, `PATCH /language`, `PATCH /speakers/{speaker_id}` (`name`; any member who can see the meeting; 404 otherwise)
- Meeting detail also returns `speech_micro_usd` and `llm_micro_usd` (the meeting's cost so far) to admins only; a member's response
  has neither field. The report page shows it under Details.
- Members (admin only, 403 otherwise): `GET /api/members`, `POST /api/members/invites` (email, role; returns the
  sign-in link and `email`: `sent`, `off` with no `[mail]` relay, or `failed`; the invite is saved either way), `DELETE /api/members/invites/{email}`, `PATCH /api/members/{id}` (role), `DELETE /api/members/{id}`.
  Removing the last admin, removing yourself or changing your own role is refused.
- Workspaces: `GET /api/workspaces` (accepts pending invites for the person's email, then lists id, name, role and which
  one is current), `POST /api/workspaces/{id}/switch` (403 without a membership; sets the session's workspace and the one
  used at the next sign-in). `GET /api/me` includes `workspace { id, name }`.
- `POST /api/ask` (`question`, optional `from`, `to` as UTC dates and `speaker`; a filter keeps only meetings in that range where that person spoke, never widens access, and leaves documents out), documents, settings, usage, calendar connect. The list above is a guide, not exhaustive: the
  router is `crates/server/src/routes.rs`. Not found there on 2026-11-25: `GET /api/events`, participant purge (*unverified*: only a text search of `routes.rs`).

## Language models

Two adapters (Anthropic Messages, OpenAI-compatible Chat Completions) behind a `Provider` trait and a
per-task `Router`. Reports are written once and stored. Ask sends only top retrieved chunks. Outputs are
JSON validated against a schema.

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
Deletes are hard deletes, including search rows and audio files. After a purge the database zeroes deleted rows, folds the search index and truncates the write-ahead log; a weekly job (19:00 to 22:00 UTC) vacuums each workspace database. A meeting can be deleted by its owner or
by an admin, and not while it is joining, recording or processing.

## Provider keys and outages

- Deployment-wide keys live in the environment file, never in `config.toml`, logs or the database.
- Deletes are hard deletes on our side; Recapz makes no provider-side delete calls.
- If a provider is unavailable, audio stays on disk and jobs retry; there is no automatic second provider.
