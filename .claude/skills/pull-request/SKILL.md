---
name: pull-request
description: Use for the full PR lifecycle: branch, push, open, merge checklist and post-merge local cleanup.
---

# Pull request

Naming, commit, title, description and merge-message conventions live in `CONTRIBUTING.md`; autonomy and the
critical-change rules live in the delivery pipeline doc. This skill adds only the gates and the order.

## Open

1. Branch from a fresh `origin/main`. Rename a tool-suggested branch name before the first push.
2. Gate before pushing: a `test-runner` `STATUS: PASS` and a `code-reviewer` `APPROVE` for the current HEAD with
   `Dirty: no`. Docs-only changes (Markdown and `CLAUDE.md` only) skip both; anything under `.claude/`, config or
   scripts is code. If HEAD moved since the gate, stop. This skill never runs tests.
3. Run the size and commit checks `CONTRIBUTING.md` lists. Over 800 source lines: split, or ask for `size-exception`.
4. Push and open the PR ready for review (not draft). Body in bullets: Summary (one per change), Testing (what ran
   and what did not), a one-line reason per new dependency, and "Decisions needing the maintainer" (exactly `None`
   unless one is open; the auto-merge check refuses a PR without it).
5. Any code change after the gate re-runs `test-runner` and the reviewer.

## Merge

Ticket PRs merge through `ship-ticket`. For any other PR, merge only when all hold:

- Open, not draft, targets `main`, mergeable.
- CI passes on the HEAD. No CI (no checks and no runs about 2 minutes after the last push) means the local PASS
  stands in.
- Reviewer approved or every blocker is fixed (docs-only skips), size is within limits, conventions hold.
- The change is not critical, and `scripts/automerge-check.sh` says `ELIGIBLE` for this HEAD, or gives only
  non-critical denylisted-path reasons that you name in the audit comment.

Merge with the explicit squash message from `CONTRIBUTING.md`, plus `--delete-branch` and
`--match-head-commit <the sha you checked>`. Never `--admin`, `--auto`, a merge commit, a rebase merge, or any
flag that skips checks.

## After the merge

- Fast-forward local `main` to `origin/main`; never commit on `main`.
- Delete the local branch with `-D` (git never sees a squash-merged branch as merged), the remote branch if GitHub
  did not, and any worktree used, leaving `git worktree list` clean.
