# Review notes

Running record for the maintainer: assumptions made while building, open issues, blockers and questions.
Add a line when you decide something non-obvious. Newest entries at the bottom of each section.

## Assumptions
- Repo is the private `tungmai-agilityio/recapz`; commits use the `tungmai-agilityio` account via `GH_TOKEN`.
- Branches are `<type>/<name>`, commits and PR titles are Conventional Commits, PRs are squash-merged (CONTRIBUTING.md).
- CI is removed for now (#6). Checks run locally: `make lint test`, `scripts/check-commits.sh`, `scripts/pr-size.mjs`.
- License: proprietary, all rights reserved; no LICENSE file.
- Rust is pinned to 1.99.1 (`rust-toolchain.toml`); web uses pnpm 12.15.0 and Node LTS.
- Week 1 was split into small PRs; PR #1 is the original single-PR version and is closed unmerged.
- A meeting is visible to its owner, or to everyone in the workspace when `share = 'workspace'`. Attendee-based sharing arrives with calendar sync.
- Job runner only claims job kinds it has a handler for; uploads queue a `transcribe` job that waits until the speech work lands.
- Sign-in uses Google OAuth code flow plus the userinfo endpoint (not ID-token verification) and requires a verified email.
- `POST /auth/dev-login` exists for local development and is off unless `dev_login = true` in `config.toml`.

## Open issues
- Google sign-in has not been exercised against real Google credentials.
- `BANNED_NAMES` is not set anywhere, so `scripts/banned-names.sh` skips. The list must be provided outside the repo.
- Dependabot opened major bumps (#12 tower-http 0.7, #13 toml 1.x, #14 @types/node 26). Not reviewed or merged.
- Repo settings still allow merge commits and rebase merges (earlier PRs landed as merge commits); squash-only is not enforced. Branch protection needs a paid plan for private repos.

## Blockers
- `gh pr merge` is denied by the session's permission system even with an allow rule, so PRs are opened as a stack and merged by the maintainer.

## Questions for review
- Which speech model and Zoom join path to choose depends on real recordings (needs team recordings with consent).
