---
name: ship-ticket
description: Implement one ticket issue end to end in its own worktree - claim, branch, implement, test, review, fix, open the PR, watch CI, merge it when every auto-merge condition holds, otherwise escalate. Runs without asking the maintainer. Use when the maintainer names a ticket (or says "ship the next ticket"). Stage 4 of docs/specs/delivery-pipeline.md.
---

# Ship one ticket

One ticket, one PR, from claim to merge without asking the maintainer anything and without an interactive question
tool. The delivery pipeline doc ("Autonomy and escalation", "Merge") is the rulebook for deciding, merging and
escalating; this skill is the order of work. The ticket is the plan: do not write another one.

**Never stop for** a failure under the retry caps, a reviewer nit, a point the ticket, spec, decisions or code
settles, CI that reports nothing, or a denylisted-path reason on a non-critical change.

## 1. Claim

- Check whether auto-merge is paused (`scripts/automerge-pause.sh detect`). If so, still ship, but hand the PR to
  the maintainer at step 5.
- No ticket given: take the highest-priority, lowest-numbered open `status:ready` ticket whose blockers are closed.
- Refuse (comment, leave it ready) if its `Touches` overlap a `status:in-progress` ticket, or three are in progress.
- Move the label from `status:ready` to `status:in-progress`. Work in your own worktree off `origin/main`, on a
  branch named as `CONTRIBUTING.md` says.
- Read the ticket, its spec, `CLAUDE.md` and every file in `Touches`.

## 2. Implement

- Build only the Acceptance list, inside `Touches`, following `CLAUDE.md`. Settle unclear points by the pipeline's
  decision order and record each under "Decisions made".
- **Drift check** before every test run: diff size and changed paths against `origin/main`. Over about 600 source
  lines, a path outside `Touches`, or a spec that proves wrong is drift: escalate. Never widen the PR.

## 3. Test and review

Every new commit restarts this loop; a PASS or APPROVE counts only for the HEAD it names. Docs-only skips it.

1. `test-runner` with the worktree and the ticket's Testing scope. On FAIL fix the code, never the test.
   **Max 3 failed fix cycles.** A non-reproducing infrastructure failure is re-run once and not counted.
2. On PASS with `Dirty: no`, `code-reviewer` with the ticket number and the PASS report. Fix blockers and go back
   to 1; fix a nit only if trivial. **Max 2 CHANGES REQUESTED rounds.**
3. Done at `APPROVE` with `Risk:` and `Reviewed:` for the current HEAD.

## 4. Open the PR and watch CI

- Open it with the `pull-request` skill. `Closes #<n>` only for the whole ticket, else `Refs #<n>` and a new ticket
  for the rest. Fill "Decisions made" and "Decisions needing the maintainer" (only an unsettled, costly-to-undo
  choice, or exactly `None`). The ticket stays in progress.
- Watch CI in the background, never in the foreground. CI failure: read the log, fix, back to step 3 (counts as a
  fix cycle; an infrastructure failure is re-run once uncounted). No CI result: proceed with `--local-pass`.

## 5. Merge or hand over

Grade your own risk by the pipeline's critical-change rules (a doubt is checked in the spec and code, not graded
`high`), then run `scripts/automerge-check.sh` with the reviewed sha, both risk grades and, without CI,
`--local-pass`.

- `ELIGIBLE`, or only denylisted-path reasons on a non-critical change: merge as the `pull-request` skill says.
  Post an audit comment starting `Auto-merged:` with the CI jobs (or the local substitute), reviewed commit, both
  grades, overridden paths if any, and "Decisions: None". Set the ticket `status:done` and clean up.
- Anything else: comment the reasons on the PR and escalate.
- After a merge, check CI on `main` once. If this PR broke it, file a `type:bug` `priority:high` ticket and ship it;
  revert only if that fix fails.

## Escalate

Set the ticket `needs-review` with a comment (what you tried, the last failure or drift, the branch), leave the
branch unpushed unless a PR exists, and send the maintainer one push notification. Only the maintainer sets the
ticket back to ready.

## Report

The PR and whether it merged, CI or "no CI result", fix cycles and review rounds used, and any escalation reason.
