# Recapz

Private meeting notetaker. Shared engineering contract for this repository.

**Sources of truth:** Product: `docs/spec.md`, prototype, `docs/decisions/`. Delivery: `docs/specs/delivery-pipeline.md`. Git: `CONTRIBUTING.md`.

## Architecture

One Rust binary (axum) serves an embedded React + Vite + TypeScript + Tailwind app (`rust-embed`).

- `crates/server`: HTTP routes, config, job runner; only binary.
- `crates/core`: domain, control/per-tenant SQLite (`rusqlite`), migrations, job queue.
- `crates/speech`: audio decoding, `Transcriber`; hosted provider, Deepgram first.
- `crates/llm`: `Provider`, per-task `Router`, cost accounting; OpenAI default, Anthropic and OpenAI-compatible adapters. Versioned prompts in `crates/llm/prompts`.
- `crates/notetaker`: browser-free logic, opt-out detection, speaker matching.
- `crates/eval`: speech/prompt evaluation (`cargo run -p eval`).
- `web/`: pages in `src/pages`, primitives in `src/ui`, tokens in `src/index.css`.

## Engineering Rules

- Inspect existing code and follow established patterns. Change only necessary files; no unrelated refactoring or reformatting.
- Every database read must enforce current-member meeting access.
- Migrations are additive only; user-data deletion must be a hard delete.
- New dependencies require a one-line PR justification.
- Prompt changes require `cargo run -p eval` and a prompt version bump.
- Frontend: follow the approved prototype (including mobile and both skins), reuse `web/src/ui`, and use design tokens, never hex colors.
- Notetaker host: run Chromium on `notetaker-profile/` only as `recapz` (a root-owned file breaks every call's profile copy: "dropped before enough audio"), and keep the `BrowserSignin: 0` policy (without it Chromium signs the Google account out on every start).
- Use Recapz terminology consistently: notetaker, meeting report, transcript, speaker ribbon, Ask, live view, upload, members, recording & privacy.

## Testing & Verification

- Setup: `make hooks` once. Development: `make dev` (server `:8080`, Vite `:5173`).
- Every PR: `cargo fmt --check` and Clippy. Scope the tests as below.
- Rust tests: `cargo test --workspace` for core, migrations, auth, meeting access, or shared API changes; otherwise test each changed crate.
- Web changes: `pnpm -C web test` and frontend lint. Browser tests for shared UI or cross-page flows.
- Tests must assert exact values/counts and fail when their guard is removed. Never disable or weaken tests to pass.
- UI changes tick their rows in `docs/ui-parity.md` with prototype screenshots; an unticked row needs a written reason.
- Before PR: `test-runner` PASS on current HEAD, then `code-reviewer`. Docs-only PRs (Markdown and `CLAUDE.md` only; `.claude/`, config and scripts count as code) skip both and state `docs only, no tests run`.

## Git Workflow

Follow `CONTRIBUTING.md`.

- One task, one concern, one PR, in dependency order; split milestones into task-sized PRs.
- Target ~300 source lines (small), ~600 (medium); over 800 requires `size-exception`.
- Branch from `origin/main`; squash merge. Check size with `node scripts/pr-size.mjs origin/main`.
- Push only your own branch. Never push to `main`, force-push shared branches, or use `--no-verify`.
- Use `--force-with-lease` when necessary.
- Merging and escalation: "Autonomy and escalation" in `docs/specs/delivery-pipeline.md`. Never `--admin`, `--auto` or a merge commit.

## Security & Privacy

Non-negotiable:

- No AI/assistant references in commits, trailers, author names, or branches; no `Co-Authored-By`.
- Never commit secrets, `config.toml`, recordings, or real meeting data. Logs contain IDs, never transcript text.
- Sessions/cookies: `HttpOnly`, `SameSite`, `Secure` behind HTTPS. State-changing routes require CSRF protection. Secrets never reach logs or browsers.
