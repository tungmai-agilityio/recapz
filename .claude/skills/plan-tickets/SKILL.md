---
name: plan-tickets
description: Turn a merged spec's ticket table into GitHub ticket issues with the full label set, blockers and the fixed body, then close the intent. Use after a spec PR has merged, or when the maintainer says "plan tickets" for a spec. Stage 3 of docs/specs/delivery-pipeline.md.
---

# Plan tickets from a merged spec

Input: a spec file in `docs/specs/`. Output: one `kind:ticket` issue per row of its Tickets table, then the intent
issue closed. Never invent tickets that are not in the table; if the table is wrong, stop and escalate.

Use `gh` as the account that has access to this repository. If several accounts are logged in, set `GH_TOKEN` for the command; never run `gh auth switch`.

## Checks before creating anything

1. The spec is on `origin/main` (`git fetch origin main && git cat-file -e origin/main:docs/specs/<file>`). An
   unmerged spec is not approved: stop.
2. Its `Intent: #N` issue exists and is `kind:intent`. Tickets for this spec do not already exist
   (`gh issue list --search "<spec file name>" --state all`); if some do, stop and say which.
3. Every label the plan needs exists (`gh label list`): run `scripts/labels.sh` if any is missing.
4. Each row has: title as `type(scope): summary`, area, blocked by, touches, size, testing scope. A row over about
   600 source lines, or missing a field, is sent back: set the intent to `status:needs-human` with a comment and stop.

## Create

For each row, in table order (so blockers exist first):

- Title: the row's title. Labels: `kind:ticket`, `status:ready`, `type:<type>`, `priority:<p>` (default
  `priority:p2`), and one `area:` per area, the first being the commit scope. Refuse a ticket missing any required
  label.
- Body, with exactly these headings (same as `.github/ISSUE_TEMPLATE/ticket.yml`): Spec (link to the file on
  `main`), Goal, Acceptance (a checkable list taken from the spec), Touches (one path per line), Blocked by
  (`#N` issue numbers of earlier tickets, or `None`), Size, Testing scope.
- Map the table's row numbers in "Blocked by" to the issue numbers you just created.

Then add the created issue numbers to the spec's Tickets table in a docs-only PR (`docs/<feature>-tickets`,
title `docs(specs): link <feature> tickets`) with the `pull-request` skill, and close the intent:
`gh issue close <intent> --comment "Tickets: #a, #b, #c"` after the last ticket exists.

## Stop and escalate

On any failure part-way, do not delete issues already created. Set the intent to `status:needs-human` with a
comment listing what exists and what is missing, and stop.

## Next

Unless the maintainer said "spec only", continue with the `ship-epic` skill for this spec (`CLAUDE.md`, Autonomy).
