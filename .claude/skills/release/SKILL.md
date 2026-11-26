---
name: release
description: Cut a release on demand - pick the next v0.x version, build notes from Conventional Commit titles merged since the last tag, create the GitHub release after the maintainer confirms, and print the upgrade checklist. Use when the maintainer says "release" or "cut a release". Stage 5 of docs/specs/delivery-pipeline.md.
---

# Release

Never touch the production host, its config or secrets. The maintainer runs the install. Install and macOS app
details live in `docs/deploy.md`.

## Steps

1. **Releasable `main`:** CI passed on `origin/main`'s HEAD; otherwise stop and name the failed job. Open
   `needs-review` tickets are reported, not blocking.
2. **Range:** since the last `v0.*` tag (none yet: whole history, `v0.1.0`). Nothing merged since: stop.
3. **Version (stays `v0`):** any `feat` or breaking `!` bumps the minor, anything else the patch.
4. **Notes** from the merged subjects (read the type after the `T-<NNN>` code, keep the code in the line): breaking
   changes first, then Features, Fixes (`fix`, `perf`), Other (summarised if long), each with its `(#N)`. No AI
   mentions.
5. **Upgrade checklist:** the deploy doc's upgrade steps plus what the range changed: new migrations (count them),
   new or changed config keys and environment variables, changes under `deploy/`, new system requirements.
6. **Confirm** version, notes and checklist with the maintainer before creating anything. On a yes, create the
   GitHub release targeting `main` (it creates the tag; never push tags by hand). On a no or silence, create nothing.
7. **macOS app:** the newest release must always carry a `latest.json`, or the app's update check gets a 404.
   Build and upload it when `desktop/` changed since the last tag or the previous release has no `latest.json`,
   otherwise copy the previous release's
   `latest.json`, both as `docs/deploy.md` describes. Never read or print the updater key. Not on the maintainer's
   Mac: say the release has no `latest.json` yet and stop; never skip silently.
8. Print the checklist as the install commands to run, and stop. Do not run them.

## Escalate

Failing CI on `main`, an existing tag, or notes that cannot be built: say why and create nothing.
