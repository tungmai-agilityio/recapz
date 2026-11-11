# Contributing

## Branches
- `main` is always releasable. Work happens on short-lived branches, one per task.
- Name: lowercase kebab-case, optionally prefixed by an area: `meetings/token-bucket-limit`, `fix-upload-limit`.
- Delete the branch after merge (local and remote).

## Commits
- Subject: `area: imperative summary`, at most 72 characters, no trailing period.
  Areas are crate or app names and topics: `server`, `core`, `web`, `meetings`, `deploy`, `docs`, `ci`.
- Body (optional): wrap at 72, explain why, not what.
- Example:

  ```
  meetings: add token-bucket limit to public API

  Limit is 100 requests per minute per key with bursts to 150.
  Over-limit responses return 429 with a Retry-After header.
  ```
- Clean-room rules apply to everything: no AI mentions, no `Co-Authored-By` trailers, no other product names.
  See `CLAUDE.md`.

## Pull requests
- One concern per PR; keep it reviewable. Title uses the same `area: summary` format.
- Description: what changed, how it was tested, and a one-line reason for any new dependency.
- CI must pass: fmt, clippy, tests, web lint/test/build, commit-message check, banned-names check.
- Merge with **squash**; the PR title becomes the commit subject on `main`.
- Run `make lint test` locally before pushing.
