---
name: ship-ticket
description: Implement one ticket issue end to end in its own worktree - claim, branch, implement, test, review, fix, open the PR, watch CI, merge it when every auto-merge condition holds, otherwise escalate. Runs without asking the maintainer. Use when the maintainer names a ticket (or says "ship the next ticket"). Stage 4 of docs/specs/delivery-pipeline.md.
---

# Ship one ticket

Input: a ticket issue number, or none for "the next ticket". One ticket is one PR. The run goes from claim to
merge without asking the maintainer anything and without an interactive question tool ("Autonomy and escalation"
in `docs/specs/delivery-pipeline.md`). It stops only at "Escalate".

**Never stop for:** a test or CI failure under the retry caps, a reviewer nit, a point the ticket, spec,
`docs/decisions/` or existing code settles, CI that reports nothing, or a denylisted-path reason on a change that is
not critical. The ticket is the plan: do not write a separate one.

## 1. Set up and claim

- Run `scripts/automerge-pause.sh detect`. If auto-merge is paused, still ship the ticket, but hand the PR to the
  maintainer at step 6. When the detected revert is of a PR whose ticket you hold, reopen that ticket as
  `needs-review` with a comment.
- No number given: list open `status:ready` ticket issues (titles without `intent:`), sort by `priority:high` first
  through `priority:low`, then lowest number, and skip any whose "Blocked by" lists an open issue. Stop if none.
- Refuse the ticket (comment, leave it `status:ready`) if its `Touches` paths overlap an open `status:in-progress`
  ticket, or three tickets are already `status:in-progress`.
- Claim: `gh issue edit <n> --remove-label status:ready --add-label status:in-progress`.
- In the main checkout, create your own worktree: `git fetch origin main && git worktree add -b <type>/<short-name>
  .claude/worktrees/<short-name> origin/main`, and work only there. Branch name as the `pull-request` skill says;
  `type` from the `type:` label (`bug` is `fix`; for `task` use the kind the ticket title names).
- Read the issue, the linked spec in `docs/specs/`, `CLAUDE.md` and every file in `Touches`.

## 2. Implement

- Build only what the ticket's Acceptance list asks, inside `Touches`. Follow the web rule in `CLAUDE.md`
  (prototype first, `web/src/ui/`, `docs/ui-parity.md`) and the test rules under "Testing & Verification".
- Decide anything unclear by the order in "Autonomy and escalation" and write it under "Decisions made" in the PR.
- **Drift check**, before every `test-runner` run: `node scripts/pr-size.mjs origin/main` and
  `git diff --name-only origin/main`. Over about 600 source lines, a path outside `Touches`, or a spec that turns
  out wrong is drift: go to "Escalate". Never widen the PR.
- Commit with a `type(scope): summary` subject (`pull-request` skill).

## 3. Test and review loop

Every new commit restarts this loop on the new HEAD; a PASS or APPROVE counts only for the HEAD it names.

1. `test-runner` with the worktree folder and the ticket's Testing scope. On FAIL fix the code, never the test.
   **At most 3 failed fix cycles**, then "Escalate". A failure that looks like infrastructure (network, runner,
   cache) and does not reproduce is re-run once and is not a cycle.
2. On `STATUS: PASS` with `Dirty: no`, run `code-reviewer` with the ticket number and the PASS report. Fix
   blockers, then back to 1. A nit is fixed only if trivial (then back to 1) and never counts. **At most 2
   CHANGES REQUESTED rounds** for blockers, then "Escalate".
3. Done when the reviewer returns `APPROVE`, `Risk:` and `Reviewed: <sha>` for the current HEAD.

## 4. Open the PR

With the `pull-request` skill: title `type(scope): summary`, body ending `Closes #<n>` only if the PR delivers the
whole ticket, otherwise `Refs #<n>` (and create a ticket for the missing part first). Fill "Decisions made" with
each choice you settled, and "Decisions needing the maintainer" with only a choice the spec does not settle that is
costly to undo, or exactly `None`. The ticket stays `status:in-progress`.

## 5. Watch CI

- Run `gh pr checks <pr> --watch` in the background (or Monitor), never in the foreground.
- **No CI:** if about 2 minutes after the last push `gh pr checks <pr>` says "no checks reported" and
  `gh run list --commit <HEAD sha>` is empty, CI reports nothing. That is not a failure: go to step 6 with
  `--local-pass`.
- **CI failed:** read the log (`gh run view <id> --log-failed`), fix, push, and go back to step 3; it counts as a
  test-fix cycle. An infrastructure failure is re-run once (`gh run rerun <id> --failed`) without counting.
- **CI green** on the current HEAD: step 6.

## 6. Merge or hand over

Grade the risk yourself, `low` or `high`. `high` only if the change is critical under "Autonomy and escalation":
an always-critical item, or a meeting-access, migration or deletion change that the spec and the ticket's
Acceptance do not decide. A spec-approved change of that kind is `low`. A doubt is checked in the spec and the
code, not graded `high`. Then run:

`scripts/automerge-check.sh <pr> --reviewed-sha <full sha the reviewer approved> --implementer-risk <low|high> --reviewer-risk <low|high, or na for a docs-only PR> [--local-pass <full sha of the test-runner PASS>]`

- `ELIGIBLE`: `gh pr merge <pr> --squash --delete-branch --match-head-commit <that sha>` (never `--auto`,
  `--admin` or a flag that skips checks). Post the audit comment: it starts with `Auto-merged:` and lists the CI
  jobs (or "no CI result: local test-runner PASS and code-reviewer APPROVE on <sha>"), the reviewed commit, both
  risk grades and "Decisions: None". Swap the closed issue's `status:` label for `status:done`. Clean up as the
  `pull-request` skill says, including the worktree.
- `ESCALATE`: if every reason is a denylisted path and the change is not critical, merge anyway with the same
  command and name the overridden paths in the audit comment. Any other reason (paused, `needs-review`, high risk,
  open decisions, CI or HEAD problems, unreadable data), or a critical change: comment the reasons on the PR, set
  the ticket `needs-review`, and stop.
- After a merge, check CI on `main` once if CI reports (`gh run list --branch main --limit 3`); a run cancelled by
  a newer merge does not count. If it failed because of this PR, file a `type:bug` `priority:high` ticket linking
  this one and ship it; revert only if that fix fails.

## Escalate

Set `needs-review` on the ticket with a comment: what you tried, the last failure or the drift found, and the
branch name. Leave the branch unpushed unless a PR already exists. Send the maintainer one push notification naming
the ticket and the reason. Only the maintainer sets the ticket back to `status:ready`.

## Report

End with: the PR and whether it merged, CI or "no CI result" (so a stopped CI gets fixed), test-fix cycles and
review rounds used, and any escalation reason.
