# Delivery pipeline

Status: **design agreed with the maintainer (2026-11-17); nothing is built.** Rollout steps below are separate
PRs, each one concern.

## Problem

Specs and plans live in `docs/`, but there is no intake, no ticket tracking and no link between a plan row and
the PR that ships it. Status is hand-maintained Markdown and drifts. This defines one path from idea to release
and who owns each step.

## Pipeline

| Stage | Entry point | Output | Gate |
|---|---|---|---|
| Intake | `idea` issue template | Issue `kind:idea`, `status:backlog` | none |
| Grill | `grill-spec` skill | `docs/specs/<feature>.md` in a docs-only PR with `Refs #<idea>`; idea is `status:planning` | **maintainer merges the spec PR** |
| Plan | `plan-tickets` skill | One issue per ticket, `status:ready`; closes the idea once every ticket exists | none |
| Ship | `ship-ticket` skill | Implement, test, verify, review, open the PR | auto-merge or escalate (below) |
| Deliver | `release` skill, on demand | `v0.x` tag, GitHub release, upgrade checklist | asks before tagging |

- Each stage is a skill in the project's skill directory, started by the maintainer. No CI orchestrator.
- A spec PR is always merged by the maintainer; merging it is the approval, and tickets are created only after.
- Spec files use one template: problem, decisions, out of scope, acceptance, ticket list. `docs/spec.md` stays
  the product spec and is amended by the PR that ships the feature. The older `docs/*-plan.md` files move into `docs/specs/`, and the decision records into
  `docs/decisions/`, in a separate PR; they are not rewritten to the template.

## State: GitHub Issues

Issues own status; Markdown holds specs and decisions only. A PR says `Closes #N`, so merging closes the ticket.

| Namespace | Values | Rule |
|---|---|---|
| `kind:` | `idea`, `ticket` | exactly one |
| `status:` | `backlog`, `planning`, `ready`, `in-progress`, `review`, `needs-human` | exactly one on an open issue |
| `type:` | `feat`, `fix`, `refactor`, `chore`, `docs`, `perf`, `test` | exactly one per ticket; gives branch prefix and commit type. `ci`, `build`, `revert` use `chore` |
| `priority:` | `p0`, `p1`, `p2`, `p3` | exactly one; default `p2`. `p0` is security, data loss or a broken `main` |
| `area:` | `server`, `core`, `web`, `auth`, `jobs`, `meetings`, `deploy` | at least one; the first is the commit scope |
| `human-merge` | n/a | forces a manual merge |

`done` is a closed issue. `blocked` is computed from open `blocked by #N` links, never stored.
`scripts/labels.sh` creates every label idempotently and is the source of truth; `plan-tickets` refuses a
ticket missing a required label.

### Label transitions

| Event | Owner | Change |
|---|---|---|
| Idea filed | template | `status:backlog` |
| Grill starts | `grill-spec` | `status:planning` |
| Spec PR merges | n/a | idea stays `status:planning` (the PR uses `Refs`, not `Closes`, so a merged spec with no tickets is never hidden) |
| Tickets created | `plan-tickets` | tickets `status:ready`; idea closed after the last ticket exists |
| Ticket claimed | `ship-ticket` | `status:in-progress` |
| PR opened | `ship-ticket` | `status:review` |
| PR merges | `Closes #N` | ticket closed (only when the PR delivers the whole ticket, see Tickets) |
| Auto-merged PR reverted | pause rule | ticket reopened as `status:needs-human` with a comment |
| Any stop or escalation | any skill | `status:needs-human` plus a comment saying why |
| Escalation cleared | maintainer only | back to `status:ready` |

## Tickets

- One ticket is one PR, within the 300/600 line guide. The planner splits anything it estimates over about 600.
- Fixed body: Spec link, Goal, Acceptance (checkable list), `touches:` paths, `blocked by #N`, size
  (small/medium), Testing scope (crates or web, per the merge rule in `CLAUDE.md`).
- A PR says `Closes #N` only when it delivers the whole ticket. A PR that delivers part says `Refs #N`, and the
  missing part gets its own ticket before the merge.
- Work found missing after a ticket closed gets a new `type:fix` ticket linking the original. Reopen a ticket
  only when its PR was reverted.
- Order: `priority:p0` first, then p1, p2, p3, then lowest issue number, skipping tickets with an open blocker.

## Ship

- Sequence per ticket: implement, `test-runner`, `code-reviewer`, PR. The existing test and review helpers are reused; the reviewer
  also checks the diff against the ticket's acceptance list and grades risk.
- **Parallel work:** the maintainer starts one worktree per ticket. `ship-ticket` claims a ticket by setting
  `status:in-progress` and refuses one whose `touches:` overlap another `in-progress` ticket. At most 3 at once.
- **Retry caps:** 3 failed test-fix cycles or 2 reviewer CHANGES REQUESTED rounds stop the ticket with
  `status:needs-human` and the last failure in a comment.
- **Drift:** if the spec is wrong or the ticket too big, stop, comment, set `status:needs-human`. Never deviate
  silently or widen the PR. A spec amendment goes through a docs-only PR, then the ticket resumes.

## Merge

A ticket PR merges without the maintainer only when **all** hold:

1. The required CI check is green on the current HEAD.
2. `code-reviewer` returns APPROVE with zero blockers for that HEAD, and every acceptance item is met.
3. The PR's "Decisions needing the maintainer" section is empty. Anything touching privacy, data deletion or
   data sent to language-model providers escalates regardless.
4. Both the implementer and the reviewer grade the risk as low; any disagreement escalates.
5. The path denylist does not match: `crates/core`, migrations, auth, meeting access, shared API types,
   `crates/llm/prompts`, dependency manifests and lockfiles, `deploy/`, `scripts/`, the project's skill and
   helper directory, `.github/`, `CLAUDE.md`. The denylist can only force escalation, never grant a merge.
6. No `human-merge` label.

Otherwise the PR gets `status:needs-human` and the maintainer merges or decides. Merging is a squash merge;
the guards against pushing to `main` are unchanged.

**Safety net:** every auto-merged PR gets a comment listing what was checked. If an auto-merged PR is
reverted, auto-merge pauses until the maintainer resumes it. Docs-only spec PRs and anything on the denylist
are never auto-merged.

**Accepted risk:** risk is judged by the implementer and reviewer, not only by script, and auto-merge starts on
the full non-risky set with no trial period. CI covers lint and tests, not behaviour; the acceptance checks
carry that weight.

## Deliver

`release` runs on demand: picks the next `v0.x` tag, builds notes from the Conventional Commit titles merged
since the last tag grouped by `feat`/`fix`, creates the GitHub release after asking, and prints the upgrade
checklist from `docs/deploy.md`. The maintainer runs the install; automation never touches the host or secrets.

## Rules that still hold

Text outside the skill and helper directory (issues, specs, PRs, commits, branches, labels) follows the
clean-room rules in `CLAUDE.md`. Every other hard rule is unchanged.

## Rollout

Auto-merge stays off until steps 2, 5 and 6 have merged.

0. Verify `test-runner` finds `cargo` in a worktree; fix if not; share one `CARGO_TARGET_DIR` across worktrees.
1. Issue templates, `scripts/labels.sh`, CONTRIBUTING section, PR template field "Decisions needing the
   maintainer".
2. Minimal CI (fmt, clippy, `cargo test`, web test and lint) and branch protection requiring it. Confirm the
   repository plan supports required checks and auto-merge.
3. Spec template and `grill-spec`.
4. `plan-tickets`.
5. `ship-ticket`, the reviewer's acceptance and risk grading, merge-when-green. Updates the `pull-request`
   skill, which currently forbids `--auto` and requires approval per merge, and the `CLAUDE.md` line "The
   maintainer approves each merge".
6. Denylist script, audit comment, `human-merge` and pause on revert.
7. `release`.

Then run the pipeline on the next real feature.

## Out of scope

Project boards, a batch command that starts workers for every ready ticket, parallelism above 3, automated
deployment, migrating existing plan files.

## Open questions

- Whether the repository plan allows required checks and auto-merge (check in step 2).
- Where the pause-on-revert flag lives (repo file or label); decide in step 6.
