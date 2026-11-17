# Recapz

Private meeting notetaker for one team on one server. A notetaker joins Google Meet and Zoom, transcribes
mixed Vietnamese and English on our own GPU server, writes a meeting report, and lets the team search every
meeting with cited answers. One Rust binary (axum) with an embedded React + Vite + TypeScript + Tailwind app.

Status: early development, building toward v1. See `docs/spec.md` for the feature spec.

## Layout
- `crates/` — Rust workspace (server and core crates; more are added as features land)
- `web/` — React app
- `docs/` — spec and decisions
- `scripts/` — local checks (`check-commits.sh`, `pr-size.mjs`)
- `.claude/` — agents, hooks and skills for coding sessions

## Develop
Prerequisites: Rust (version pinned in `rust-toolchain.toml`), Node LTS and pnpm. Copy
`config.example.toml` to `config.toml` and fill it in; `config.toml` is never committed.

```
make dev      # server on :8080, web dev server on :5173
make test     # Rust and web tests
make lint     # fmt, clippy, web lint
```

## Contributing
Read `CONTRIBUTING.md` (branch, commit and PR rules) and `CLAUDE.md` (clean-room rules). Never commit
`config.toml`, keys, recordings or real meeting data.

## License
Proprietary. All rights reserved.
