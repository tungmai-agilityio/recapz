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
- Calendar sync is Google-only and reads upcoming events in a window; events keep `auto_join` and the booked meeting across re-syncs. The join rule is `manual` by default, `auto` books one notetaker per link.
- Join links are parsed strictly (Meet and Zoom hosts only) and stored normalised, so the same call is never joined twice by one member.
- Zoom v1 uses the Meeting SDK bot per the decision doc in #29; it needs Zoom app review, and upload stays the fallback.
- Report email transport is undecided: assume SMTP behind a trait, nothing is wired yet.
- Weeks 6-7 PRs stack on `chore/integration-wk6`, which merges the week 3-5 stacks; the order is in the merge order below.

## Open issues
- Google sign-in has not been exercised against real Google credentials.
- `BANNED_NAMES` is not set anywhere, so `scripts/banned-names.sh` skips. The list must be provided outside the repo.
- Dependabot opened major bumps (#12 tower-http 0.7, #13 toml 1.x, #14 @types/node 26). Not reviewed or merged.
- No CSRF token on state-changing routes yet (`POST /auth/logout`, `POST /api/uploads`); only `SameSite=Lax` cookies protect them. Add before any public exposure.
- PRs #15-#21 are open for review as a stack (#16 → #18 → #19 → #20 → #21; #15 and #17 independent).
- Repo settings still allow merge commits and rebase merges (earlier PRs landed as merge commits); squash-only is not enforced. Branch protection needs a paid plan for private repos.
- `join_rule` is stored but no sync caller reads it yet, and the settings API for it is deferred.
- The join API dedupes per member only; two members pasting the same link each get a notetaker. Cross-owner dedupe is not done.
- A second member who can see a call has no access to the meeting a first member booked until it is shared.
- Google Calendar responses are not capped in size or page count yet.

## Blockers
- `gh pr merge` is denied by the session's permission system even with an allow rule, so PRs are opened as a stack and merged by the maintainer.
- Zoom Meeting SDK bot needs Zoom app review and an account owner for the app; nothing can join a Zoom call until then.
- Google Calendar needs an OAuth scope for calendar read-only and somewhere to store refresh tokens per member; sync parsing is tested, the live calls are not.
- Real Meet and Zoom calls (and so `scripts/smoke-join.sh`) need test calls kept open and a member cookie; the live join path has not run.
- Report email needs a transport decision (SMTP assumed).

## Merge order (weeks 6-7)
- #57 (join links) and #58 (calendar events) first, into `chore/integration-wk6`; then #59 (upcoming API), #60 (Google parsing), #61 (join scheduler) in that order, then the smoke-test PR #62.
- The week 2 stack (to #39) and the LLM stacks (#42-#56) merge through their own integration branches; this file will conflict with #46/#56, so keep both sets of lines when resolving.

## Questions for review
- Which speech model and Zoom join path to choose depends on real recordings (needs team recordings with consent).
