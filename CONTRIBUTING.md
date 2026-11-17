# Contributing

## Branches
- `main` is always releasable. Work happens on short-lived branches, one per task, cut from `origin/main`.
- Name: `<type>/<short-kebab-description>`, using the same types as commits: `feat/google-sign-in`,
  `fix/upload-size-limit`, `chore/ci-cache`, `docs/spec-update`. No user or tool prefixes.
- Delete the branch after merge (local and remote).
- Run `make hooks` once per clone: the pre-push hook refuses pushes to `main`.
  Rewrite your own branch with `git push --force-with-lease`, never `--force`.

## Commits (Conventional Commits)
- Subject: `type(scope): imperative summary`, at most 72 characters, no trailing period. Scope is optional.
- Types: `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `ci`, `perf`, `build`, `revert`.
  Add `!` after the scope for a breaking change.
- Scopes are an area of the code: `server`, `core`, `web`, `auth`, `jobs`, `meetings`, `notetaker`, `speech`, `llm`,
  `deploy`, `tooling` (hooks, scripts, `.github`, dependency updates).
- Body (optional): wrap at 72, explain why, not what.
- Example:

  ```
  feat(meetings): add token-bucket limit to public API

  Limit is 100 requests per minute per key with bursts to 150.
  Over-limit responses return 429 with a Retry-After header.
  ```
- Clean-room rules apply to everything: no AI mentions, no `Co-Authored-By` trailers, no other product names.
  See `CLAUDE.md`.

## Issues and tickets
- Issues own task status; `docs/specs/` and `docs/decisions/` hold specs and decisions. The full flow, labels and
  merge rules are in `docs/specs/delivery-pipeline.md`.
- Start from an **Intent** issue. A merged spec turns into **Ticket** issues, each one PR.
- Every ticket carries one `kind:`, `status:`, `type:` and `priority:` label (default `priority:p2`) and at least
  one `area:` label. `type:` matches the branch prefix and commit type; the first `area:` is the commit scope.
- `Closes #N` in a PR only when it delivers the whole ticket; otherwise `Refs #N`.
- Labels come from `scripts/labels.sh` (safe to re-run); change them there, not by hand in GitHub.

## Pull requests
- One concern per PR; keep it reviewable. Title uses the same `type(scope): summary` format.
- Description: what changed, how it was tested, and a one-line reason for any new dependency.
- CI is paused for now. Before opening a PR run `make lint test`,
  `./scripts/check-commits.sh origin/main..HEAD "<PR title>" "<branch>"` and `node scripts/pr-size.mjs origin/main`.
- Merge with **squash**; the PR title becomes the commit subject on `main`.
- Run `make lint test` locally before pushing.
