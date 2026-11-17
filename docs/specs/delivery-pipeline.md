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
| Grill | `grill-spec` skill | `docs/specs/<feature>.md` in a docs-only PR with `Refs #<idea>`; idea is `status:planning` | the maintainer's confirmation that ends the grilling |
| Plan | `plan-tickets` skill | One issue per ticket, `status:ready`; closes the idea once every ticket exists | none |
| Ship | `ship-ticket` skill | Implement, test, verify, review, open the PR | auto-merge or escalate (below) |
| Deliver | `release` skill, on demand | `v0.x` tag, GitHub release, upgrade checklist | asks before tagging |

- Each stage is a skill in the project's skill directory, started by the maintainer. No CI orchestrator.
- The maintainer's approval of a spec is the confirmation that ends the grilling. `grill-spec` then merges its
  docs-only spec PR itself once CI is green, and tickets are created only after that merge.
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
| `area:` | `server`, `core`, `web`, `auth`, `jobs`, `meetings`, `notetaker`, `speech`, `llm`, `deploy`, `tooling` | at least one; the first is the commit scope. `tooling` covers hooks, scripts, `.github` and dependency updates |
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
| PR merges | `Closes #N`, then `ship-ticket` | ticket closed (only when the PR delivers the whole ticket, see Tickets); its `status:` label is removed |
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
- **Parallel work:** until `ship-epic` exists (rollout step 8), the maintainer starts one worktree per ticket. `ship-ticket` claims a ticket by setting
  `status:in-progress` and refuses one whose `touches:` overlap another `in-progress` ticket. At most 3 at once.
- **Retry caps:** 3 failed test-fix cycles or 2 reviewer CHANGES REQUESTED rounds stop the ticket with
  `status:needs-human` and the last failure in a comment.
- **Drift:** if the spec is wrong or the ticket too big, stop, comment, set `status:needs-human`. Never deviate
  silently or widen the PR. A spec amendment goes through a docs-only PR, then the ticket resumes.

## Merge

`CLAUDE.md` (Merging and escalation) is the rule: PRs merge themselves and only critical changes go to the
maintainer. This section is how a ticket PR is checked. It merges without the maintainer when **all** hold:

1. Every CI check passed on the current HEAD, read by the merge step itself with `gh pr checks`: no pending, failed or
   skipped job. Editing a PR's title or labels must not replace the heavy results, so `ci.yml` (rust, web, guards)
   does not run on those edits and `pr-meta.yml` (commits, size) does.
2. `code-reviewer` returns APPROVE with zero blockers for that HEAD, and every acceptance item is met.
3. The PR's "Decisions needing the maintainer" section is empty. Anything touching privacy, data deletion or
   data sent to language-model providers escalates regardless.
4. Both the implementer and the reviewer grade the risk as low; any disagreement escalates.
5. The path denylist does not match: `crates/core`, `crates/server`, `crates/llm` and `crates/eval` (meeting
   access, API handlers, prompts and model routing), migrations, `web/src/api.ts` (shared API types), any path
   containing auth, access, session, csrf, privacy or purge, dependency manifests, lockfiles and toolchain files,
   `deploy/`, `scripts/`, `Makefile`, `config.example.toml`, the project's skill and helper directory, `.github/`,
   `CLAUDE.md`, `CONTRIBUTING.md`, `docs/specs/` and `docs/decisions/`. Both names of a renamed file are checked.
   The denylist is a signal and never grants a merge by itself: a path-only reason for a change that is not critical
   under `CLAUDE.md` may be overridden, naming the overridden paths in the audit comment. `scripts/automerge-lib.sh`
   holds the list.
6. No `human-merge` label.

Otherwise, and for any critical change, the PR gets `status:needs-human` and the maintainer merges or decides. Merging is a squash merge;
the guards against pushing to `main` are unchanged.

**How the merge runs:** `scripts/automerge-check.sh` reads every condition above and prints `ELIGIBLE` or the
reasons to escalate; any failure to read data counts as a reason. The caller passes the full reviewed commit id and
both risk grades. `ship-ticket` then runs `gh pr merge --squash --match-head-commit <that id>` itself, so a push
after the check cannot slip in. Never `--auto`. Nothing server-side stops someone with push access from
merging a red PR by hand.

**Safety net:** every auto-merged PR gets a comment listing what was checked. If an auto-merged PR is
reverted, auto-merge pauses until the maintainer resumes it. The audit comment starts with `Auto-merged:`; the
pause flag is the repository variable `AUTOMERGE_PAUSED`, set by `scripts/automerge-pause.sh`, which detects a
merged revert PR that mentions an auto-merged PR. A variable that cannot be read counts as paused. Critical changes (`CLAUDE.md`, Merging and
escalation) are never merged without the maintainer.

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
2. Minimal CI (fmt, clippy, `cargo test`, web test and lint). The repository is private on the free plan, where
   branch protection, rulesets and GitHub's own auto-merge are unavailable (checked 2026-11-17: the API returns
   403 and `allow_auto_merge` is false), so the check is enforced by the merge step, see Merge.
3. Spec template and `grill-spec`.
4. `plan-tickets`.
5. `ship-ticket`, the reviewer's acceptance and risk grading, merge-when-green. Updates the `pull-request`
   skill, which currently forbids `--auto` and requires approval per merge, and the `CLAUDE.md` line "The
   maintainer approves each merge".
6. Denylist script, audit comment, `human-merge` and pause on revert.
7. `release`.
8. `ship-epic <spec>`, built only after several single-ticket runs have merged cleanly (see Epic runs).

Then run the pipeline on the next real feature.

## Epic runs

`ship-epic <spec>` runs a spec's tickets in the background without the maintainer starting each one. It stops
only on `status:needs-human`; the spec merge and any denylist merge still need the maintainer.

- Reads the spec's tickets, applies the order rule, the `touches:` overlap check and the cap of 3, and starts
  one worker per eligible ticket in its own worktree; when one finishes it starts the next.
- A ticket or PR that needs the maintainer is parked as `status:needs-human`. The run skips it and continues
  with tickets that do not depend on it, and notifies the maintainer each time it parks one.
- Halts at a configured ticket or spend limit, and pauses and retries on rate limits or CI outages instead of
  failing the ticket. Ends with a summary: merged, parked, and tickets blocked behind a parked one.
- A ticket chain behind a parked ticket waits; the run does not work around a dependency.

## Out of scope

Project boards, parallelism above 3, automated deployment, migrating the contents of existing plan files.

## Open questions

- Move to GitHub Pro (required checks, branch protection) if a second committer joins.
