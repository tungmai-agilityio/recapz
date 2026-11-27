# Audio download

Status: **shipped (2026-11-27).** #704 to #706 merged. Not verified in a running app: the save in Chrome, Safari and the desktop app, and download speed from the object store.
Intent: #703. Docs ticket: #704.

## Problem

A member can play a meeting's audio on the report page but cannot save it. `GET /api/meetings/{id}/audio` already
streams from local disk or the object store with `Range` support and the meeting access rule, so the missing piece is
a download action with a readable file name.

## Decisions

- **Same route, `?download=1`:** `GET /api/meetings/{id}/audio?download=1` returns the same body as playback plus a `Content-Disposition: attachment` header. Without the parameter the response is unchanged.
- **Same access as playback:** `db::MEETING_VISIBLE`. A member who cannot see the meeting, and a member of another workspace, get 404. No new permission.
- **Original format, no transcode:** the bytes on disk (or in the bucket) are sent as they are. The extension comes from the stored file.
- **File name `<title>-<YYYY-MM-DD>.<ext>`:** the date is `started_at` when set, else `created_at`, in UTC. The header carries an ASCII `filename=` (marks stripped, `"`, `\`, `/`, control characters and newlines replaced) and a UTF-8 `filename*=UTF-8''<percent-encoded>` that keeps Vietnamese characters. The title is never written raw into a header.
- **409 while `joining` or `recording`:** the file is still being written. Playback is unchanged in those states.
- **404 when there is nothing to send:** no audio, a meeting the member cannot see, or audio removed by the retention sweep.
- **Object store streamed through the server:** no presigned URL, so the bucket stays private and the access rule is applied on every request.
- **Log line:** one line per download with the meeting ID and user ID. Never the title or the file name.
- **Report page ⋯ menu only:** a "Download audio" item, shown when the meeting has audio and is not `joining` or `recording`. It is a plain link with the `download` attribute, so the browser saves the file without loading it into memory.
- **Downloaded copies leave Recapz's control:** a saved file is not removed by audio retention, by opt-out deletion or by deleting the meeting. The Recording & privacy copy says so (text lands with #706).

## Out of scope

- Transcoding or format choice, video, presigned URLs.
- An audit table of downloads (the log line is the only record).
- A download button on the meetings list.

## Acceptance

- [ ] With `download=1` the response has `Content-Disposition: attachment; filename="<ascii>"; filename*=UTF-8''<percent-encoded>`; a title with Vietnamese marks, `"`, `\`, `/` and a newline produces the exact expected header, with no header injection.
- [ ] Without `download` the response is the current playback response (no `Content-Disposition`).
- [ ] `Range` with `download=1` returns 206 with the exact bytes and `Content-Range`; the object-store path sends the same header.
- [ ] A member who cannot see the meeting, and a member of another workspace, get 404; so does a meeting whose audio the retention sweep removed.
- [ ] `joining` or `recording` with `download=1` gets 409; playback is unchanged.
- [ ] The log line holds IDs only.
- [ ] "Download audio" is in the report ⋯ menu only when the meeting has audio and is not `joining` or `recording`, and the file saves with the expected name in Chrome and Safari.
- [ ] Recording & privacy says downloaded copies are not removed by retention or deletion.

## API

`GET /api/meetings/{id}/audio?download=1`: same status codes and `Range` behaviour as playback, plus:

| Case | Result |
|---|---|
| Visible meeting with audio, ready | 200 or 206, `Content-Disposition: attachment; filename="..."; filename*=UTF-8''...` |
| `joining` or `recording` | 409 |
| Not visible, other workspace, no audio, removed by retention | 404 |

## UI

Prototype: https://claude.ai/artifact/KxQaWLV4L9kusPFz1eonzH (Report, updated 2026-11-27): a ⋯ menu in the report header
with Rename, Download audio and Delete, in both skins and on mobile. Parity row: `docs/ui-parity.md` (Report),
unticked until #706.

## Unverified

- Whether the Tauri desktop app (WKWebView) saves a link with the `download` attribute. #706 added an `on_download` handler (own origin only, saves to ~/Downloads); it compiles but has not been run. Fallback: open the URL in the default browser.
- Download speed of a long recording streamed from the object store through the droplet. Playback is chunked in 8 MiB ranges; a plain download streams the whole object.

## Tickets

| # | Title | Area | Blocked by | Touches | Size | Testing scope |
|---|---|---|---|---|---|---|
| #704 | docs(spec): plan audio download | docs | none | `docs/specs/audio-download.md`, `docs/spec.md`, `docs/README.md`, prototype | small | docs only |
| #705 | feat(server): download a meeting's audio | server | #704 | `crates/server/src/routes.rs`, `crates/server/tests/api.rs` | small | `cargo test -p recapz-server`, fmt, clippy |
| #706 | feat(web): download audio from the report | web | #705 | `web/src/pages/Report.tsx`, `web/src/pages/Recording.tsx`, `docs/ui-parity.md` | small | `pnpm -C web test`, frontend lint |

## Open questions

None.
