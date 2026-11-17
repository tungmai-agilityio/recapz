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
- make hooks    # once per clone: pre-push hook that refuses pushes to main
- make dev      # server on :8080 + vite on :5173
- make test     # all Rust and web tests
- make lint     # cargo fmt --check, clippy -D warnings, pnpm lint
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
- Web work: match the prototype first. Start from the screen's prototype state (layout, components, mobile, both skins), use the shared primitives in `web/src/ui/`, then wire data. `docs/ui-parity.md` lists what is left.
- Branch per task from `origin/main`: `git fetch origin && git switch -c <type>/<short-name> origin/main`. Never commit on `main`.
- Tickets: plan them with the `plan-tickets` skill and ship each with `ship-ticket` (docs/specs/delivery-pipeline.md).
- Docs-only PRs (Markdown and `CLAUDE.md` only, no code, config or scripts) skip `test-runner` and `code-reviewer`; say "docs only, no tests run" under Testing.
- Implement, then `test-runner` once when done (give it the folder and scope). On FAIL fix the code, never the test; re-run only if code changed since the last PASS. On `STATUS: PASS` for the current HEAD, run `code-reviewer` (docs-only PRs skip it), fix blockers, then open the PR with the `pull-request` skill. `explore-code` for broad searches.
- Branches `<type>/<name>`, commits and PR titles `type(scope): summary` (Conventional Commits, CONTRIBUTING.md). Merge your own PRs under "Merging and escalation" below. After a merge, clean up the local branch as the `pull-request` skill says.
- `gh`: never `gh auth switch`; pass the work account's token per command (see `pull-request`).

## Merging and escalation

Work autonomously. Merge your own PR (squash, `--match-head-commit` on the commit you checked) once CI is green on
the current HEAD, `code-reviewer` has approved that HEAD (docs-only PRs skip the reviewer) and the PR body has no
open decision. Do not wait for the maintainer.

Escalate only when the change is critical: leave the PR open, set the ticket `status:needs-human` with a comment
saying why, and notify the maintainer. Critical means any of:
- it breaks, or needs to bend, a hard rule or the clean-room rules;
- meeting access, sign-in, sessions, secrets, or anything that decides who can see data;
- data leaving the server, data deletion, or a migration that rewrites or drops data;
- a production deploy, a new dependency with a doubtful licence or security record, or a new system requirement;
- the guard rails themselves: `CLAUDE.md`, the push and test guards, `scripts/git-hooks/`, the auto-merge scripts, CI;
- a product decision the spec does not make, or a spec that turns out to be wrong;
- CI still red or reviewer blockers still open after the retry caps in `docs/specs/delivery-pipeline.md`.

`scripts/automerge-check.sh` is a signal, not the rule: read its reasons. A reason that is only a denylisted path,
for a change that is not critical by the list above, may be merged anyway; name the overridden paths in the audit
comment. Every other reason (paused, `human-merge`, high risk, open decisions, CI or HEAD problems, unreadable
data) escalates. Never use `--admin`, `--auto`, a merge commit or any flag that skips checks, and never push to
`main`.

## Hard rules

A PR that breaks any of these, or a Conventions rule above, is rejected.

- The clean-room rules above, in every file, commit, branch and PR text.
- Never skip, disable or delete a test to make CI pass.
- Never push to `main` or force-push a shared branch; on your own branch use `--force-with-lease` only. Never use `git commit --no-verify`.
- Never commit `config.toml`, API keys, recordings or real meeting data. Logs hold IDs, never transcript text.
- Only transcript text, and text of documents the uploader marked "May be sent to the language model", go to LLM and embedding providers; audio never leaves the server.
- A web PR ticks its screen's rows in docs/ui-parity.md and attaches a screenshot next to the prototype state; an unticked row needs a written reason.

## Things Claude gets wrong

Mistakes code review caught more than twice. Add one line per mistake, phrased as "Do not …" with the fix.

- Do not put a whole milestone in one PR; split it into task-sized PRs of one concern each, in dependency order.
- Do not write a test that would still pass if the feature were broken; assert exact values, assert counts before looping, and check the test fails with the guard removed.
