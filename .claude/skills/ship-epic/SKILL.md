---
name: ship-epic
description: Run all open tickets of one spec in the background - start up to 3 ticket workers at once, park anything that needs the maintainer, keep going on independent tickets, stop at a budget, and end with a summary. Use when the maintainer says "ship the epic" or names a spec to run. Stage 6 of docs/specs/delivery-pipeline.md.
---

# Ship an epic

Input: a spec whose tickets exist, plus optional limits `max-tickets` (default 10 started this run) and
`max-parallel` (default 3, never above 3). Workers implement and merge; this skill schedules them.
No worker asks the maintainer anything; the delivery pipeline doc decides what escalates.

## Before starting

Refuse, saying what is missing, unless the spec is on `origin/main`, its tickets exist, and CI on `main` is green.
Then run `scripts/automerge-pause.sh detect`.

## Loop

1. A ticket is eligible when it is `status:ready`, its blockers are closed, and its `Touches` do not overlap an
   in-progress ticket. Order by priority, then lowest number.
2. Within both limits, start one background worker per eligible ticket in its own worktree, prompted to use the
   `ship-ticket` skill and report the PR number and ELIGIBLE or ESCALATE.
3. When a worker finishes, record its result and go back to 1.
4. A ticket that ends `needs-review`, or a PR left for the maintainer, is **parked**: never retry or work around
   it; its dependants wait. Send one push notification per parked ticket.
5. A rate limit or CI outage is not a failure: wait (scheduled wake-up, 20 minutes or more) and retry the same
   ticket without counting it.
6. Stop when nothing is eligible, the budget is used, or auto-merge pauses after a revert (finish PRs in flight,
   start nothing new).

## Summary

Tickets merged (PR numbers), parked (with reasons), blocked behind parked ones, not started because of the budget,
and budget used. Never call the epic complete unless every ticket of the spec is closed.
