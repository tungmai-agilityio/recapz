# Recapz

Private meeting notetaker. One Rust binary (axum) with an embedded React + Vite + TypeScript + Tailwind app.
SQLite per tenant via rusqlite. Speech via whisper-rs. LLM via crates/llm (Anthropic + OpenAI-compatible adapters).

## Clean-room rules (mandatory)
- Never mention AI, assistants or Claude in commit messages, trailers, author names or branch names.
  Do not add Co-Authored-By trailers.
- Never name any other meeting, notes or productivity product anywhere in the repo:
  code, identifiers, comments, docs, tests, fixtures, sample data, commits, branches.
- Use Recapz feature names only (notetaker, meeting report, transcript, speaker ribbon, Ask, live view,
  upload, members, recording & privacy).
- Do not copy UI text, layouts, prompts, API shapes or data formats from other products.
  Build only from docs/spec.md and the Recapz prototype. Ask the maintainer when unsure.

## Commands
- make dev      # server on :8080 + vite on :5173
- make test     # all Rust and web tests
- make lint     # cargo fmt --check, clippy -D warnings, pnpm lint, banned-names check
- make release  # production build with CUDA

## Conventions
- Git branch, commit and PR rules: see CONTRIBUTING.md (`area: summary` commits, squash merge).
- Every DB read checks meeting access for the current member.
- Migrations are additive only. Deletions of user data are hard deletes.
- New dependencies need a one-line reason in the PR description.
- Prompts live in crates/llm/prompts with a version number; run `cargo run -p eval` before changing them.
