# Recapz

Private meeting notetaker. One Rust binary (axum) with an embedded React + Vite + TypeScript + Tailwind app.
SQLite per tenant via rusqlite. Speech via a hosted provider (Deepgram first, behind a `Transcriber` trait). LLM and embeddings via crates/llm (OpenAI default; Anthropic and any OpenAI-compatible adapter).

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
- `gh`: use the account that has access to this repository; if several are logged in, set `GH_TOKEN` per command and never `gh auth switch` (see `pull-request`).

## Autonomy

A spec the maintainer confirmed, and the tickets planned from it, are approved work. Do not ask the maintainer for
review, confirmation or a choice while shipping them. After `plan-tickets`, go on with `ship-epic` in the same
session unless the maintainer said "spec only".

When something is unclear, decide in this order and record it under "Decisions made" in the PR: the ticket's
Acceptance, the spec, `docs/decisions/`, an existing pattern in the code, then the smallest reversible option.
Never use the interactive question tool or end a turn with a question inside `ship-ticket` or `ship-epic`;
stop only for the conditions under "Merging and escalation".

## Merging and escalation

Work autonomously. Merge your own PR (squash, `--match-head-commit` on the commit you checked) once CI is green on
the current HEAD, `code-reviewer` has approved that HEAD (docs-only PRs skip the reviewer) and the PR body has no
open decision. Do not wait for the maintainer.

Escalate only when the change is critical: leave the PR open, set the ticket `status:needs-human` with a comment
saying why, and notify the maintainer. Then go on with independent tickets. Critical means any of:

*Always critical* (a spec cannot pre-approve these):
- it breaks, or needs to bend, a hard rule or the clean-room rules;
- sign-in, sessions, CSRF or secrets;
- data going to a destination the hard rules do not already allow;
- a migration that rewrites or drops data, or a deletion of user data the spec does not name;
- a production deploy, a new dependency with a doubtful licence or security record, or a new system requirement;
- the guard rails themselves: `CLAUDE.md`, the push and test guards, `scripts/git-hooks/`, the auto-merge scripts, CI;
- the change deviates from the spec, the spec turns out to be wrong, or a product decision is needed that the spec
  does not make;
- CI still red or reviewer blockers still open after the retry caps in `docs/specs/delivery-pipeline.md`.

*Critical only when the spec does not decide it*: meeting access rules (who can see a meeting), transcript text,
questions or marked documents going to the language-model and embedding providers, an additive migration, and
deleting user data. If the spec decides it, the ticket's Acceptance names it and `code-reviewer` confirms the diff
matches, it is routine: merge it.

`scripts/automerge-check.sh` is a signal, not the rule: read its reasons. A reason that is only a denylisted path,
for a change that is not critical by the list above, may be merged anyway; name the overridden paths in the audit
comment. Every other reason (paused, `human-merge`, high risk, open decisions, CI or HEAD problems, unreadable
data) escalates. Never use `--admin`, `--auto`, a merge commit or any flag that skips checks, and never push to
`main`.

## Hard rules

A PR that breaks any of these, or a Conventions rule above, is rejected.

- The clean-room rules above, in every file, commit, branch and PR text.
- Commits are authored and committed with the work email (`@asnet.com.vn`) or the work account's GitHub noreply address, never a personal one; `make hooks` enforces it. In a clone outside `~/Work`, set `user.name` and `user.email` locally.
- Never skip, disable or delete a test to make CI pass.
- Never push to `main` or force-push a shared branch; on your own branch use `--force-with-lease` only. Never use `git commit --no-verify`.
- Never commit `config.toml`, API keys, recordings or real meeting data. Logs hold IDs, never transcript text.
- Audio goes only to the configured speech provider. Only transcript text, the text of a question a member asks, and text of documents the uploader marked "May be sent to the language model", go to the configured LLM and embedding providers. Providers are named to members, must run with no-training and no-retention settings, and nothing else leaves the server.
- A web PR ticks its screen's rows in docs/ui-parity.md and attaches a screenshot next to the prototype state; an unticked row needs a written reason.

## Things Claude gets wrong

Mistakes code review caught more than twice. Add one line per mistake, phrased as "Do not …" with the fix.

- Do not put a whole milestone in one PR; split it into task-sized PRs of one concern each, in dependency order.
- Do not write a test that would still pass if the feature were broken; assert exact values, assert counts before looping, and check the test fails with the guard removed.
