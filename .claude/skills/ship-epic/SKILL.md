---
name: ship-epic
description: Run all open tickets of one spec in the background - start up to 3 ticket workers at once, park anything that needs the maintainer, keep going on independent tickets, stop at a budget, and end with a summary. Use when the maintainer says "ship the epic" or names a spec to run. Stage 6 of docs/specs/delivery-pipeline.md.
---

# Ship an epic

Input: a spec file in `docs/specs/` whose tickets exist (created by `plan-tickets`), plus optional limits:
`max-tickets` (default 10 tickets started in this run) and `max-parallel` (default 3, never above 3).
The run stops only on `needs-review`; the spec merge and every critical PR ("Autonomy and escalation" in `docs/specs/delivery-pipeline.md`) still need the maintainer. No worker asks the maintainer a question; see "Autonomy and escalation" in `docs/specs/delivery-pipeline.md`.

## Before starting

Refuse and say what is missing if any of these is false:
1. The spec is on `origin/main` and its tickets exist: `gh issue list --search "<spec file>"` (skip titles with `intent:`).
2. `scripts/automerge-check.sh`, `scripts/automerge-pause.sh` and the `ship-ticket` skill exist.
3. CI on `main` is green. Run `scripts/automerge-pause.sh detect`.

## Loop

1. List the spec's open tickets. A ticket is eligible when it is `status:ready`, none of its "Blocked by" issues is
   open, and its `Touches` paths do not overlap an `status:in-progress` ticket. Order: `priority:high` first through
   `priority:low`, then lowest number.
2. While fewer than `max-parallel` tickets are `status:in-progress` and fewer than `max-tickets` have been started,
   start a worker per eligible ticket: the Agent tool with `isolation: "worktree"`, running in the background,
   with the prompt "Use the ship-ticket skill for ticket #<n>. Report the PR number and ELIGIBLE or ESCALATE."
3. When a worker finishes, record its result and go back to step 1.
4. A ticket that ends `needs-review`, or a PR left for the maintainer, is **parked**: do not retry or work
   around it. Tickets blocked behind it wait. Send the maintainer a push notification naming the ticket and the
   reason, once per parked ticket.
5. A rate limit or a CI outage is not a failure: wait with a scheduled wake-up (20 minutes or more) and retry the
   same ticket; do not count it against the ticket's retry caps.
6. Stop when no eligible ticket remains, the budget is reached, or `scripts/automerge-pause.sh status` says paused
   after a revert (finish the PRs in flight and start nothing new).

## Summary

End with: tickets merged (PR numbers), parked (with reasons), blocked behind parked ones, not started because of
the budget, and the budget used. Do not claim the epic is complete unless every ticket of the spec is closed.
