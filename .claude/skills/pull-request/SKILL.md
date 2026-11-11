---
name: pull-request
description: Use for the full PR lifecycle: branch, push, open, merge checklist and post-merge local cleanup, including the gh account rule.
---

# Pull request

## Rules for `gh` and branches

- Several GitHub accounts may be logged in. Never change the shared active account (`gh auth switch`); pass the work account's token per command: `GH_TOKEN=$(gh auth token --user tungmai-agilityio) gh ...`, including in sub-agent briefs. (A `.envrc` above this folder may already export it; do not rely on that in new shells or sub-agents.)
- The maintainer approves each merge before you run it.
- Docs-only PRs skip `code-reviewer`.
- Branch names are kebab-case with an area prefix, `<area>/<short-name>` (`server/health-endpoint`, `core/job-queue`, `docs/git-convention`), matching the PR title's area. Never a user prefix or a tool-suggested name: rename the branch before the first push.
- Clean-room rules (CLAUDE.md) apply to branch names, titles, bodies and commits: no AI mentions or tool trailers, no other product names.
- One task, one concern, one PR. Do not deliver a milestone in one PR.

## Create

1. `git fetch origin main && git switch -c <area>/<short-name> origin/main`
2. Before pushing: confirm the handoff cites a `test-runner` `STATUS: PASS` for the current HEAD with `Dirty: no`, and a `code-reviewer` `APPROVE` (docs-only PRs skip the reviewer); if HEAD moved since, stop and return to the main session. This skill never runs tests. Then `node scripts/pr-size.mjs origin/main`. Over 800 source lines: stop and split, or ask for the `size-exception` label.
3. `git push -u origin <branch>`, then `gh pr create` (ready for review, not draft; `gh pr edit` to update) with the title `area: summary` (at most 72 characters, no trailing period); it becomes the squashed commit on `main`.
4. Body sections, in plain words: Summary (what and why), Testing (what ran and what did not), and a one-line reason for every new dependency.
5. If review fixes change the code after the PASS, re-run `test-runner` and the reviewer before the merge.

## Merge

Given a PR number, check every item before merging:

1. `gh pr view <n> --json state,isDraft,baseRefName,mergeable,labels,title` — must be open, not draft, target `main`, and be mergeable.
2. `gh pr checks <n>` shows every check passing, and the recorded `test-runner` PASS matches the PR's HEAD.
3. The PR body confirms `code-reviewer` approved, or that every blocker raised is fixed (docs-only PRs skip this).
4. Size is within the guide (about 300 small, 600 medium, warning above 800 source lines), or the PR has the `size-exception` label.
5. The title is `area: summary` and no commit carries a tool trailer.
6. The maintainer said yes to this merge, in the thread or session where the work runs. Approval for one PR does not cover another.

Only if all hold, run `gh pr merge <n> --squash --delete-branch`. Never use `--admin`, `--auto`, a merge commit or rebase merge, or any flag that skips checks.

## After the merge

The squash commit on `main` is the only record; the branch is spent. Clean up the local checkout:

1. `git switch main && git fetch origin main --prune && git merge --ff-only origin/main` (fast-forward only, never commit on `main`).
2. `git branch -D <branch>` (squash merges are never seen as merged by `git branch -d`).
3. Delete the remote branch only if GitHub did not already: `git push origin --delete <branch>`.
4. If a worktree was created: `git worktree remove <path>`, `git worktree prune`, and check `git worktree list` shows no leftovers.
