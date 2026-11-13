# Running Recapz locally

One command once the setup below is done: `make local` (server with real transcription on :8080, web app on :5173).

## Setup (once)

1. Install Rust (version pinned in `rust-toolchain.toml`), Node and pnpm, then `brew install cmake` (builds the speech engine).
2. Download a ggml speech model. For a quick local test the small multilingual one (about 470 MB) is enough; the default `large-v3-turbo` is 1.6 GB.
   ```sh
   mkdir -p models && curl -L -o models/ggml-small.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
   ```
3. `cp config.example.toml config.toml` (never commit it) and set:
   - `speech_model = "./models/ggml-small.bin"`
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

Open http://localhost:5173. Without `[llm]` the app still transcribes but writes no reports.

## Sign in (dev login)

Open http://localhost:5173. On the sign-in screen the dev server shows a "Dev login (local only)" form: enter the `bootstrap_admin_email` and press Sign in. The form is not in production builds. If it says rejected, `dev_login = true` is missing from the `config.toml` the server loaded (restart `make local` after editing).

## Try it with a sample recording

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

## Tests

```sh
cargo test --workspace   # Rust
pnpm -C web test         # web
make lint
```
