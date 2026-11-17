---
name: ship-ticket
description: Implement one ticket issue end to end in its own worktree - claim, branch, implement, test, review, open the PR, merge it only when every auto-merge condition holds, otherwise escalate. Use when the maintainer names a ticket (or says "ship the next ticket"). Stage 4 of docs/specs/delivery-pipeline.md.
---

# Ship one ticket

Input: a ticket issue number, or none for "the next ticket". One ticket is one PR. Work in your own worktree
(the maintainer starts one per ticket; at most 3 tickets are `status:in-progress` at once).

Pass the work account's token on every `gh` call: `GH_TOKEN=$(gh auth token --user tungmai-agilityio) gh ...`.

## 1. Pick and claim

- No number given: list open `kind:ticket` + `status:ready` issues, sort by `priority:p0` first through `p3`, then
  lowest number, and skip any whose "Blocked by" lists an open issue. Stop if none.
- Refuse the ticket (comment, leave it `status:ready`) if its `Touches` paths overlap an open `status:in-progress`
  ticket, or three tickets are already `status:in-progress`.
- Claim: `gh issue edit <n> --remove-label status:ready --add-label status:in-progress`.
- Read the issue, the linked spec in `docs/specs/`, `CLAUDE.md` and any file in `Touches`.

## 2. Implement

- Branch from `origin/main` as the `pull-request` skill says: `<type>/<short-name>`, `type` from the `type:` label.
- Build only what the ticket's Acceptance list asks, inside `Touches`. Follow the web rule in `CLAUDE.md` (prototype
  first, `web/src/ui/`, `docs/ui-parity.md`) and the test rules under "Things Claude gets wrong".
- **Drift:** if the spec is wrong, the ticket is too big (over about 600 source lines) or you need a path outside
  `Touches`, stop. Do not deviate or widen the PR. Go to "Escalate".

## 3. Test, review, PR

- Run `test-runner` once when done, with the folder and the Testing scope from the ticket. On FAIL fix the code,
  never the test, and re-run only if code changed. **At most 3 failed fix cycles**, then escalate.
- On `STATUS: PASS` for the current HEAD with `Dirty: no`, run `code-reviewer` with the ticket number so it checks
  each Acceptance item and grades risk. Fix blockers. **At most 2 CHANGES REQUESTED rounds**, then escalate.
- Open the PR with the `pull-request` skill: title `type(scope): summary`, body ending `Closes #<n>` only if the PR
  delivers the whole ticket, otherwise `Refs #<n>` (and create a ticket for the missing part first). Fill
  "Decisions needing the maintainer" with every choice a reader might overrule, or exactly `None`.
  Then `gh issue edit <n> --remove-label status:in-progress --add-label status:review`.
- Wait for CI: `gh pr checks <pr> --watch`. On a CI failure fix the code and count it as a test-fix cycle.

## 4. Merge or hand over

Grade the risk yourself, `low` or `high`: `high` if the change touches privacy, data deletion, data sent to
language-model providers, meeting access, or you were unsure of anything. Then run:

`scripts/automerge-check.sh <pr> --reviewed-sha <full 40-character HEAD sha the reviewer approved> --implementer-risk <low|high> --reviewer-risk <low|high from the reviewer>`

- Prints `ELIGIBLE` (and `scripts/automerge-pause.sh status` says active): merge with
  `gh pr merge <pr> --squash --delete-branch --match-head-commit <that sha>` (never `--auto`, `--admin` or a flag that skips checks), then post the
  audit comment, which must start with `Auto-merged:` and list the CI jobs, the reviewed commit, both risk grades
  and "Decisions: None". Clean up as the `pull-request` skill says, and remove the leftover `status:` label from the
  closed issue.
- Prints `ESCALATE: ...`: do not merge. Comment the reasons on the PR, set the issue `status:needs-human`, stop.
  The maintainer merges or decides.
- Before starting any ticket, run `scripts/automerge-pause.sh detect`; if a revert of an auto-merged PR exists it pauses
  auto-merge and you hand every PR to the maintainer until they run `scripts/automerge-pause.sh resume`. When the
  revert is of the PR you are looking at, also reopen its ticket as `status:needs-human` with a comment.

## Escalate

Set `status:needs-human` on the ticket with a comment: what you tried, the last failure or the drift found, and the
branch name. Leave the branch unpushed unless a PR already exists. Only the maintainer sets the ticket back to
`status:ready`.
