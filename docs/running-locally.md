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
   - Optional, for reports and Ask, under `[llm]`: `provider = "openai"`, `base_url = "https://api.openai.com/v1"`, `api_key`, `model` (for example `gpt-4o-mini`), and the two prices. The key lives only in this file; keep it out of git and chat.
4. `pnpm -C web install`

## Run

```sh
make local
```

Open http://localhost:5173. Without `[llm]` the app still transcribes but writes no reports.

## Sign in (dev login)

There is no dev-login button. In the browser devtools console on http://localhost:5173:

```js
fetch('/auth/dev-login', {method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify({email: 'you@example.com'})}).then(() => location.reload())
```

The email must be the `bootstrap_admin_email`.

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
