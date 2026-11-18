---
name: ship-ticket
description: Implement one ticket issue end to end in its own worktree - claim, branch, implement, test, review, open the PR, merge it only when every auto-merge condition holds, otherwise escalate. Use when the maintainer names a ticket (or says "ship the next ticket"). Stage 4 of docs/specs/delivery-pipeline.md.
---

# Ship one ticket

Input: a ticket issue number, or none for "the next ticket". One ticket is one PR. Work in your own worktree
(the maintainer starts one per ticket; at most 3 tickets are `status:in-progress` at once).

Use `gh` as the account that has access to this repository. If several accounts are logged in, set `GH_TOKEN` for the command; never run `gh auth switch`.

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
- Anything else that is unclear is not drift: decide by the order in "Autonomy" in `CLAUDE.md`, never ask the
  maintainer, and write it under "Decisions made" in the PR.

## 3. Test, review, PR

- Run `test-runner` once when done, with the folder and the Testing scope from the ticket. On FAIL fix the code,
  never the test, and re-run only if code changed. **At most 3 failed fix cycles**, then escalate. A failure that
  does not reproduce locally and looks like infrastructure (network, runner, cache) is re-run once and is not a cycle.
- On `STATUS: PASS` for the current HEAD with `Dirty: no`, run `code-reviewer` with the ticket number so it checks
  each Acceptance item and grades risk. Fix blockers; a nit is fixed if trivial and otherwise ignored, and never counts as a round. **At most 2 CHANGES
  REQUESTED rounds** for blockers, then escalate.
- Open the PR with the `pull-request` skill: title `type(scope): summary`, body ending `Closes #<n>` only if the PR
  delivers the whole ticket, otherwise `Refs #<n>` (and create a ticket for the missing part first). Fill
  "Decisions made" with each choice you settled, and "Decisions needing the maintainer" with only a choice the spec
  does not settle that is costly to undo, or exactly `None`.
  Then `gh issue edit <n> --remove-label status:in-progress --add-label status:review`.
- Wait for CI: `gh pr checks <pr> --watch`. On a CI failure fix the code and count it as a test-fix cycle.

## 4. Merge or hand over

Grade the risk yourself, `low` or `high`. `high` only if the change is critical under "Merging and escalation" in
`CLAUDE.md`: an always-critical item, or a meeting-access, language-model-data, migration or deletion change that the
spec and the ticket's Acceptance do not decide. A spec-approved change of that kind is `low`. A doubt is something
to check in the spec and the code, not a reason to grade `high`. Then run:

`scripts/automerge-check.sh <pr> --reviewed-sha <full 40-character HEAD sha the reviewer approved> --implementer-risk <low|high> --reviewer-risk <low|high from the reviewer, or na for a docs-only PR>`

- Prints `ELIGIBLE` (and `scripts/automerge-pause.sh status` says active): merge with
  `gh pr merge <pr> --squash --delete-branch --match-head-commit <that sha>` (never `--auto`, `--admin` or a flag that skips checks), then post the
  audit comment, which must start with `Auto-merged:` and list the CI jobs, the reviewed commit, both risk grades
  and "Decisions: None". Clean up as the `pull-request` skill says, and remove the leftover `status:` label from the
  closed issue. Then check CI on the merge commit once (`gh run list --branch main --limit 3`); a cancelled run
  superseded by a newer merge does not count. If it failed because of this PR, file a `type:fix` `priority:p0`
  ticket linking this one and ship it; revert only if that fix fails.
- Prints `ESCALATE: ...`: if every reason is a denylisted path and the change is not critical under "Merging and
  escalation" in `CLAUDE.md`, merge anyway with the same command and name the overridden paths in the audit comment.
  Any other reason (paused, `human-merge`, high risk, open decisions, CI or HEAD problems, unreadable data), or a
  critical change, means do not merge: comment the reasons on the PR, set the issue `status:needs-human`, stop.
- Before starting any ticket, run `scripts/automerge-pause.sh detect`; if a revert of an auto-merged PR exists it pauses
  auto-merge and you hand every PR to the maintainer until they run `scripts/automerge-pause.sh resume`. When the
  revert is of the PR you are looking at, also reopen its ticket as `status:needs-human` with a comment.

## Escalate

Set `status:needs-human` on the ticket with a comment: what you tried, the last failure or the drift found, and the
branch name. Leave the branch unpushed unless a PR already exists. Send the maintainer one push notification naming
the ticket and the reason. Only the maintainer sets the ticket back to `status:ready`.
