---
name: plan-tickets
description: Turn a merged spec's ticket table into GitHub ticket issues with the full label set, blockers and the fixed body, then close the intent. Use after a spec PR has merged, or when the maintainer says "plan tickets" for a spec. Stage 3 of docs/specs/delivery-pipeline.md.
---

# Plan tickets from a merged spec

Input: a spec with a Tickets table. Output: one ticket issue per row, then the intent closed. Issue, label,
milestone and title conventions are in `CONTRIBUTING.md` and the delivery pipeline doc; follow them exactly.

**The table is the plan.** Never invent, merge or split tickets. If the table is wrong, stop and escalate.

## Check first, create nothing until all hold

- The spec is on `origin/main`. An unmerged spec is not approved.
- Its intent issue exists, and no tickets for this spec exist yet; if some do, stop and say which.
- Read the topic's existing issues, PRs and milestones fresh (milestones get renamed).
- Every needed label exists; create missing ones only through the repository's label script.
- Every row has title, area, blocked by, touches, size and testing scope, and none is over about 600 source lines.
  Otherwise set the intent `needs-review` with a comment and stop.
- More than about 10 tickets: confirm with the maintainer first.

## Create

In table order, so blockers exist before the tickets they block:

- A new milestone for this plan, never an older or finished one; set it at creation.
- Full label set at creation, starting at `status:ready`; refuse a ticket missing one.
- Body with exactly the ticket template's headings (`.github/ISSUE_TEMPLATE/ticket.yml`), Acceptance as a checkable
  list taken from the spec, and row numbers in "Blocked by" mapped to the issue numbers just created.
- Rename each issue to its ticket code right after creating it.

Then link the issue numbers into the spec's Tickets table in a docs-only PR (`pull-request` skill), and close the
intent with a comment listing the tickets.

## On failure

Never delete issues already created. Set the intent `needs-review` with a comment listing what exists and what is
missing, and stop.

## Next

Unless the maintainer said "spec only", continue with `ship-epic` for this spec.
