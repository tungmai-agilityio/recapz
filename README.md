# Recapz

Private meeting notetaker for one team on one server. A notetaker joins Google Meet (Zoom calls are recorded from the macOS app or uploaded), transcribes
mixed Vietnamese and English with a hosted speech provider, writes a meeting report, and lets the team search every
meeting with cited answers. One Rust binary (axum) with an embedded React + Vite + TypeScript + Tailwind app.

Status: early development, building toward v1. See `docs/spec.md` for the feature spec.

## Layout
- `crates/` — Rust workspace: `server`, `core`, `speech`, `llm`, `notetaker`, `eval`
- `web/` — React app
- `desktop/` — macOS app (Tauri), its own Cargo workspace; `make desktop-run` to try it against `make local`
- `docs/` — start at `docs/README.md` (index, architecture, specs, decisions)
- `deploy/` — systemd units and Caddyfile
- `scripts/` — local checks (`check-commits.sh`, `pr-size.mjs`)
- `.claude/` — agents, hooks and skills for coding sessions

## Develop
Prerequisites: Rust (version pinned in `rust-toolchain.toml`), Node LTS and pnpm. Copy
`config.example.toml` to `config.toml` and fill it in; `config.toml` is never committed.

```
make hooks             # once per clone: git hooks (no pushes to main, work email only)
make dev               # server on :8080, web dev server on :5173
make local             # same, but checks config.toml exists first
make test              # Rust and web tests
make test-browser      # notetaker tests that start a real Chromium
make lint              # fmt, clippy, web lint
make pr-check          # branch, commit and PR title checks plus the PR size guide (TITLE=... to set the title)
make release           # production build
```

The server needs `RECAPZ_SPEECH_KEY` in the environment; `make dev` does not read `.env`.

With the server running and `dev_login = true` in `config.toml`, these sign in as `bootstrap_admin_email` and
try features without a real meeting (see `docs/running-locally.md`):

```
make sample                          # upload a short spoken sample (macOS say and ffmpeg)
make live-sim                        # fake live call from a built-in script; prints the meeting id
make replay FILE=call.wav            # replay a recording as a live call through the speech provider (SPEECH_LANG=vi)
make live-stop ID=<meeting id>       # end a fake call (OPTOUT=1 deletes it)
```

The Meet notetaker joins with a saved Google sign-in. Use an account made for the notetaker, not a personal one:

```
make notetaker-login     # opens a browser window: sign in, then close it
make notetaker-logout    # delete the saved sign-in
```

## Deploy
Production runs on one Debian host (details, secrets and recovery in `docs/deploy.md`). Needs `gh` logged in.

```
make deploy                                            # build main on CI, deploy, smoke test, wait for the run
gh workflow run release-build.yml --ref main -f deploy=true   # same, without waiting
scripts/bootstrap-host.sh <ip>                         # set up a fresh server from scratch
scripts/host-notetaker-login.sh <ip>                   # sign the notetaker's Google account in on the host
```

A failed smoke test rolls the deploy back on its own. The deploy job waits for approval only while the `production` environment has a required reviewer (none now). `bootstrap-host.sh` reads `config.toml`, `recapz.env` and optional `offsite.env`, `smoke.env` from `.secrets/` (git-ignored); the Droplet, DNS and buckets stay manual.

## Contributing
Read `CONTRIBUTING.md` (branch, commit and PR rules) and `CLAUDE.md` (engineering contract). Never commit
`config.toml`, keys, recordings or real meeting data.

## License
Proprietary. All rights reserved.
