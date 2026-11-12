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
- CSRF tokens on state-changing routes: done in #74 (see weeks 11-12 below); not yet on `main`.
- PRs #15-#21 are open for review as a stack (#16 → #18 → #19 → #20 → #21; #15 and #17 independent).
- Repo settings still allow merge commits and rebase merges (earlier PRs landed as merge commits); squash-only is not enforced. Branch protection needs a paid plan for private repos.
- `join_rule` is stored but no sync caller reads it yet, and the settings API for it is deferred.
- The join API dedupes per member only; two members pasting the same link each get a notetaker. Cross-owner dedupe is not done.
- A second member who can see a call has no access to the meeting a first member booked until it is shared.
- Google Calendar responses are capped in #82 (weeks 11-12), but the HTTP fetch itself is not wired, so the cap is only enforced where `parse_page` is called.

## Weeks 11-12 assumptions (hardening)
- Built on the tip of the Ask stack (`feat/web-open-cited-moment`, #72). Chain, each PR based on the one before: #74 CSRF → #75 Ask rate limit → #77 Vietnamese keywords → #79 streaming in the llm crate → #80 streamed Ask endpoint → #81 embedder swap point. #82 (calendar size caps) is based on #60, because the Ask tip does not contain the calendar server code.
- CSRF is double-submit: `x-csrf-token` must equal the `recapz_csrf` cookie on every POST/PUT/PATCH/DELETE that carries a session, and a browser `Origin`, when sent, must equal `base_url`. The token is random, not bound to the session id; bind it if sibling subdomains ever share cookies. `/api/me` issues the cookie to sessions that began before it existed.
- Ask rate limit is 20 questions per member per minute (`ask_per_minute`), in memory per process, checked after request validation and before retrieval or any model call. A restart resets it.
- Vietnamese: the index already folds marks, so unmarked questions match. Filler is matched with marks only (unmarked forms can be content words). Decomposed Unicode input is kept as one word. Vectors stored for Vietnamese chunks before #77 still include filler and rank slightly lower until re-indexed.
- Streaming: only the Anthropic adapter streams; the OpenAI-compatible adapter delivers its whole reply as one piece through the same interface. The Ask reply is JSON, so the answer text is extracted incrementally. A reply that is invalid JSON is retried without streaming only if no text was sent; after text was sent the call fails with an `error` event.
- Embedder: `Embedder::id()` is stored per chunk (`tenant_003`), search compares vectors only within one embedder, and server start re-queues meetings holding another embedder's vectors. Config `embedder = "hash"` is the only value.
- Calendar caps: 4 MiB body, 250 events, 10,000 scanned characters of notes, 2048-byte links and page tokens.
- Systemd deploy and backups are owned by another thread, not done here.

## Blockers
- No real multilingual embedder is wired. Choosing one and tuning `MIN_SIMILARITY` needs an eval set of real questions and transcripts (the current set is synthetic); the swap point in #81 is ready for it.
- The weeks 11-12 PRs had scoped tests run by the author only; the separate test-runner and code-reviewer agents were not run, so they have had no independent review. The auto-mode permission classifier also blocked adding the review assignee and requested reviewer on the PRs.
- Streaming and the calendar caps have not been run against live provider or Google responses (no keys here).
- #73 is titled as a notes PR but its diff is the Ask code and does not include this file; check its contents before merging.
- `gh pr merge` is denied by the session's permission system even with an allow rule, so PRs are opened as a stack and merged by the maintainer.
- Zoom Meeting SDK bot needs Zoom app review and an account owner for the app; nothing can join a Zoom call until then.
- Google Calendar needs an OAuth scope for calendar read-only and somewhere to store refresh tokens per member; sync parsing is tested, the live calls are not.
- Real Meet and Zoom calls (and so `scripts/smoke-join.sh`) need test calls kept open and a member cookie; the live join path has not run.
- Report email needs a transport decision (SMTP assumed).

## Merge order (weeks 6-7)
- #57 (join links) and #58 (calendar events) first, into `chore/integration-wk6`; then #59 (upcoming API), #60 (Google parsing), #61 (join scheduler) in that order, then the smoke-test PR #62.
- The week 2 stack (to #39) and the LLM stacks (#42-#56) merge through their own integration branches; this file will conflict with #46/#56, so keep both sets of lines when resolving.

## Merge order (weeks 11-12)
- #74 → #75 → #77 → #79 → #80 → #81 after the Ask stack (#64-#72) lands; #82 after #60. `config.example.toml` and `tenant_003.sql` may need a renumber or line merge if other threads touch them.

## Questions for review
- Which speech model and Zoom join path to choose depends on real recordings (needs team recordings with consent).
