---
name: release
description: Cut a release on demand - pick the next v0.x version, build notes from Conventional Commit titles merged since the last tag, create the GitHub release after the maintainer confirms, and print the upgrade checklist. Use when the maintainer says "release" or "cut a release". Stage 5 of docs/specs/delivery-pipeline.md.
---

# Release

Never touch the production host, its config or secrets. The maintainer runs the install. Pass the work account's
token on every `gh` call: `GH_TOKEN=$(gh auth token --user tungmai-agilityio) gh ...`.

## Steps

1. **Check `main` is releasable.** `git fetch origin main --tags`. The CI jobs for `origin/main`'s HEAD must have
   passed: `gh run list --branch main --limit 3` and `gh api repos/{owner}/{repo}/commits/<sha>/check-runs`.
   Stop and say which job failed if not. Open `status:needs-human` tickets are reported, not blocking.
2. **Find the range.** Last tag: `git describe --tags --abbrev=0 --match 'v0.*' origin/main`. None yet: the range
   is the whole history and the version is `v0.1.0`. Nothing merged since the last tag: stop.
3. **Pick the version** (still `v0`): any `feat` or a breaking `!` bumps the minor (`v0.3.2` to `v0.4.0`); only
   `fix`, `perf`, `refactor`, `chore`, `docs`, `test`, `ci` bumps the patch.
4. **Build the notes** from `git log <last>..origin/main --format=%s`: groups Features (`feat`), Fixes (`fix`,
   `perf`), Other (the rest, one line summarising counts if long), each line the commit subject with its `(#N)`.
   Mark breaking changes first. Notes follow the clean-room rules: no AI mentions, no other product names.
5. **Upgrade checklist** from `docs/deploy.md` (the Upgrade lines) plus what the range changed:
   - new files under `crates/core/migrations/` (migrations run on start and are additive; say how many),
   - new or changed keys in `config.example.toml` and new environment variables,
   - changes under `deploy/` (unit files to reinstall, `systemctl daemon-reload`),
   - new system packages or features (for example Chromium, a virtual display, CUDA).
6. **Show the maintainer** the version, notes and checklist and ask for confirmation before creating anything. On a
   yes: `gh release create <version> --target main --title <version> --notes-file <file>`. This creates the tag on
   the server; do not push tags by hand. On a no or silence, create nothing.
7. Print the checklist once more as the install commands to run, and stop. Do not run them.

## Escalate

Failing CI on `main`, a tag that already exists, or notes that cannot be built: say why and stop without creating
a release.
