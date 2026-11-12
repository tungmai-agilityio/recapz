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
- PR size: node scripts/pr-size.mjs origin/main (before pushing)
- Before merge: full `cargo test --workspace` only if the PR touches `crates/core`, migrations, auth, meeting access or shared API types; otherwise `cargo test -p <crate>` for the crates changed, plus `cargo fmt --check` and clippy. Web tests (`pnpm -C web test`) and lint only when `web/` changed. Browser flows only for shared UI or a cross-page flow.

## Conventions
- Git branch, commit and PR rules: see CONTRIBUTING.md (Conventional Commits, `type/` branches, squash merge).
- Every DB read checks meeting access for the current member.
- Migrations are additive only. Deletions of user data are hard deletes.
- New dependencies need a one-line reason in the PR description.
- Prompts live in crates/llm/prompts with a version number; run `cargo run -p eval` before changing them.

## Workflow

- One task, one concern, one PR; source lines about 300 (small), 600 (medium), over 800 needs the `size-exception` label. Do not deliver a milestone in one PR.
- Branch per task from `origin/main`: `git fetch origin && git switch -c <type>/<short-name> origin/main`. Never commit on `main`.
- Implement, then `test-runner` once when done (give it the folder and scope). On FAIL fix the code, never the test; re-run only if code changed since the last PASS. On `STATUS: PASS` for the current HEAD, run `code-reviewer` (docs-only PRs skip it), fix blockers, then open the PR with the `pull-request` skill. `explore-code` for broad searches.
- Branches `<type>/<name>`, commits and PR titles `type(scope): summary` (Conventional Commits, CONTRIBUTING.md). The maintainer approves each merge. After a merge, clean up the local branch as the `pull-request` skill says.
- `gh`: never `gh auth switch`; pass the work account's token per command (see `pull-request`).

## Hard rules

A PR that breaks any of these, or a Conventions rule above, is rejected.

- The clean-room rules above, in every file, commit, branch and PR text.
- Never skip, disable or delete a test to make CI pass.
- Never push to `main` or force-push a shared branch. Never use `git commit --no-verify`.
- Never commit `config.toml`, API keys, recordings or real meeting data. Logs hold IDs, never transcript text.
- Only transcript text goes to LLM providers; audio never leaves the server.
- A web PR ticks its screen's rows in docs/ui-parity.md and attaches a screenshot next to the prototype state; an unticked row needs a written reason.

## Things Claude gets wrong

Mistakes code review caught more than twice. Add one line per mistake, phrased as "Do not …" with the fix.

- Do not put a whole milestone in one PR; split it into task-sized PRs of one concern each, in dependency order.
- Do not write a test that would still pass if the feature were broken; assert exact values, assert counts before looping, and check the test fails with the guard removed.
