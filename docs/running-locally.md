# Running Recapz locally

One command once the setup below is done: `make local` (server with real transcription on :8080, web app on :5173).

## Setup (once)

1. Install Rust (version pinned in `rust-toolchain.toml`), Node and pnpm.
2. Get a Deepgram API key and export it before starting: `export RECAPZ_SPEECH_KEY=...` (read from the environment only, never `config.toml`). The server refuses to start without it, and call and upload audio is sent to Deepgram.
3. `cp config.example.toml config.toml` (never commit it) and set:
   - `bootstrap_admin_email = "you@example.com"`
   - `dev_login = true` (local only; Google sign-in is not needed)
   - `base_url = "http://localhost:5173"`: the server rejects browser POSTs (upload, Ask) whose Origin is not `base_url`, and the browser talks to the web app on :5173. Leave the default and every upload and Ask in the browser returns 403.
   - Optional, for reports and Ask, under `[llm]`: `provider = "openai"`, `base_url = "https://api.openai.com/v1"`, `api_key`, `model` (for example `gpt-4o-mini`), and the two prices. Reasoning models (the gpt-5 family) spend `max_tokens` on thinking before the reply, so with the default 2048 some report tasks come back empty and the report fails with `invalid JSON after retry (Eof)`. Start at `max_tokens = 8192` for them (a 40-second sample already used about 3,700 thinking tokens per task, so longer meetings may need more). The key lives only in this file; keep it out of git and chat.
   - Optional, only to try Google Calendar connect: set `RECAPZ_SECRET_KEY` in the environment (never in `config.toml`) to 64 hex characters, for example from `openssl rand -hex 32`. It encrypts stored Google refresh tokens; without it Connect calendar answers 503. Add `<base_url>/auth/calendar/callback` as an authorised redirect URI in the Google client.
4. `pnpm -C web install`

## Run

```sh
make local
```

`make local` loads a `.env` file in the repo root when it exists (gitignored; for example `RECAPZ_SECRET_KEY=...`), so you need not export variables by hand.

Open http://localhost:5173. Without `[llm]` the app still transcribes but writes no reports.

## Desktop app against your local server (macOS)

With `make local` running, `make desktop-run` builds the macOS app for `http://localhost:5173`, quits a running Recapz, replaces `/Applications/Recapz.app` with the new build and opens it.

```sh
make desktop-run                  # local web app, http://localhost:5173
make desktop-run TARGET=prod      # hosted server, https://recapz.tungmai.com (what make desktop-dmg bakes in)
make desktop-run URL=http://localhost:8080   # any other server (DESKTOP_BASE_URL=... also works)
make desktop-run SKIP_BUILD=1     # reuse the last build, only reinstall and relaunch
make desktop-run HELP=1           # usage
```

The URL is baked in at build time; for a local server keep it equal to `base_url` in `config.toml`. It needs `cargo tauri` and the "Recapz Local" certificate (`docs/deploy.md`, "macOS app"), not the updater key. The build is version `0.0.0` and its update URL is a dead local port, so a published release never replaces it.

The app also notices when Zoom, Teams, Slack, Webex, FaceTime, Discord or a browser starts using the microphone and
asks "Record this meeting?" (`docs/specs/meeting-detection.md`). The menu-bar item "Detect Meetings" turns it off; the
switch and the "never ask" list are stored in `detect.json` in the app's config folder. This was only unit-tested; it is
*unverified* on a real call.

## Sign in (dev login)

Open http://localhost:5173. On the sign-in screen the dev server shows a "Dev login (local only)" form: enter the `bootstrap_admin_email` and press Sign in. The form is not in production builds. If it says rejected, `dev_login = true` is missing from the `config.toml` the server loaded (restart `make local` after editing).

## Try it with a sample recording

With `make local` running, `make sample` does all of this: it speaks the sample below, converts it and uploads it as you (`bootstrap_admin_email`). The manual steps:

```sh
say -o sample.aiff "Hi Sam, we launch on March fourteenth. Sam owns the payment testing."
ffmpeg -i sample.aiff -ar 16000 -ac 1 sample.wav
```

Upload `sample.wav` in the app, or by script (the CSRF header must equal the `recapz_csrf` cookie):

```sh
curl -s -c cj.txt -H 'content-type: application/json' -d '{"email":"you@example.com"}' localhost:8080/auth/dev-login
CSRF=$(awk '$6=="recapz_csrf"{print $7}' cj.txt)
curl -s -b cj.txt -H "x-csrf-token: $CSRF" -H "Origin: http://localhost:8080" -F title=Sample -F file=@sample.wav localhost:8080/api/uploads
```

Then look at `GET /api/meetings`, `/api/meetings/{id}/transcript` and `/report`. Ask is `POST /api/ask` with `{"question": "..."}`; it answers with citations.

## Simulate a live call (no real meeting)

With `dev_login = true` the server can play a fake call into the live view, so Ask now can be tried without joining a meeting. It uses no browser, audio or speech provider; the live view, search, your documents and the language model (`[llm]` in `config.toml`) are the real code. Upload a document first (My documents, toggle "May be sent to the language model" on) so Ask now has something to answer from.

Short forms: `make live-sim` starts the built-in sample and prints the meeting id; `make live-stop ID=<meeting id>` ends it (`OPTOUT=1` deletes it). By hand:

```sh
curl -s -c cj.txt -H 'content-type: application/json' -d '{"email":"you@example.com"}' localhost:8080/auth/dev-login
CSRF=$(awk '$6=="recapz_csrf"{print $7}' cj.txt)
# Start: the built-in sample, or your own lines; one line every interval_ms (200 to 10000, default 2000).
curl -s -b cj.txt -H "x-csrf-token: $CSRF" -H "Origin: http://localhost:5173" -H 'content-type: application/json' \
  -d '{"title":"Client call","lines":["What is the hourly rate?","And the payment terms?"],"interval_ms":3000}' \
  localhost:8080/api/dev/live-sim
# -> {"id":"<meeting id>"}. Open http://localhost:5173/meetings/<meeting id>/live (the rail and the Meetings page also show it within 15 s).
# Stop (the meeting becomes ready); add ?optout=true to behave like a chat opt-out (the meeting is deleted).
curl -s -b cj.txt -X DELETE -H "x-csrf-token: $CSRF" -H "Origin: http://localhost:5173" "localhost:8080/api/dev/live-sim/<meeting id>"
```

Add `"fail_after": 2` to the start body to see "Live transcript unavailable" after two lines. The simulated meeting stays `recording` until you stop it, so Ask now keeps working; the transcript is memory only and is gone when you stop it or restart the server. This is not a test of joining Meet, audio capture, the chat notice or real speech latency; that still needs one real call.

### Replay a recording through the real speech provider

To test the live transcript with real speech (Vietnamese quality, latency), upload an audio file (up to 30 minutes, 100 MB) as a fake call. It needs `RECAPZ_SPEECH_KEY`. The audio is streamed in real time to Deepgram for the live view, then sent again in full for the stored transcript after the call ends, so both the streaming and the batch rate are billed. Use only a recording the people in it agreed to. Short form: `make replay FILE=call.wav SPEECH_LANG=vi`.

```sh
curl -s -b cj.txt -H "x-csrf-token: $CSRF" -H "Origin: http://localhost:5173" \
  -F file=@call.wav -F language=vi -F title="Replay" localhost:8080/api/dev/live-sim/audio
# -> {"id":"<meeting id>"}; open /meetings/<meeting id>/live. Stop it with the same DELETE as above (?optout=true deletes it).
```

## Tests

Before pushing, `make pr-check` checks the branch name, the commit subjects and the PR title (`TITLE=...`, default the last commit subject), then prints the size against the 800-line guide.

```sh
cargo test --workspace   # Rust
pnpm -C web test         # web
make lint
```

## Notetaker sign-in

`make notetaker-login` opens a Chromium window: sign in the notetaker's Google account, then close the window. The sign-in is saved in `notetaker-profile/` under `data_dir` (`./data` in the example config). `make notetaker-logout` deletes it. Use a Google account made for the notetaker, not a personal one. On a server, see `docs/deploy.md`.

## Notetaker debug dump

If a call fails with `meet_ui_changed: <step>`, start the server with `RECAPZ_NOTETAKER_DEBUG=1`. A failing page step then saves `page.png` (a screenshot) and `page.json` (the page's buttons and its first 600 characters of text) in `data/notetaker-debug/<meeting id>/`. The page can show participant names, so the folder is private (0700), excluded from backups, and should be deleted after use. Nothing is written without the variable.
