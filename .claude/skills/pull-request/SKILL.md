---
name: pull-request
description: Use for the full PR lifecycle: branch, push, open, merge checklist and post-merge local cleanup, including the gh account rule.
---

# Pull request

## Rules for `gh` and branches

- Several GitHub accounts may be logged in. Never change the shared active account (`gh auth switch`); pass the work account's token per command: `GH_TOKEN=$(gh auth token --user tungmai-agilityio) gh ...`, including in sub-agent briefs. (A `.envrc` above this folder may already export it; do not rely on that in new shells or sub-agents.)
- The maintainer approves each merge before you run it.
- Docs-only PRs (Markdown and `CLAUDE.md` only, no code, config or scripts) skip `test-runner` and `code-reviewer`.
- Branch names are `<type>/<short-kebab-name>` with the Conventional Commit type (`feat/google-sign-in`, `fix/upload-size-limit`, `chore/ci-cache`, `docs/spec-update`). Never a user prefix or a tool-suggested name: rename the branch before the first push.
- Clean-room rules (CLAUDE.md) apply to branch names, titles, bodies and commits: no AI mentions or tool trailers, no other product names.
- One task, one concern, one PR. Do not deliver a milestone in one PR.

## Create

1. `git fetch origin main && git switch -c <type>/<short-name> origin/main`
2. Before pushing: confirm the handoff cites a `test-runner` `STATUS: PASS` for the current HEAD with `Dirty: no`, and a `code-reviewer` `APPROVE` (docs-only PRs skip both; check `git diff --name-only origin/main` lists only docs); if HEAD moved since, stop and return to the main session. This skill never runs tests. Then `node scripts/pr-size.mjs origin/main`. Over 800 source lines: stop and split, or ask for the `size-exception` label.
3. `git push -u origin <branch>`, then `gh pr create` (ready for review, not draft; `gh pr edit` to update) with the title `type(scope): summary` (Conventional Commits, at most 72 characters, no trailing period); it becomes the squashed commit on `main`. Commit subjects on the branch use the same format.
4. Body sections, in plain words: Summary (what and why), Testing (what ran and what did not), and a one-line reason for every new dependency.
5. If review fixes change the code after the PASS, re-run `test-runner` and the reviewer before the merge.

## Merge

Given a PR number, check every item before merging:

1. `gh pr view <n> --json state,isDraft,baseRefName,mergeable,labels,title` — must be open, not draft, target `main`, and be mergeable.
2. `gh pr checks <n>` shows every check passing, and the recorded `test-runner` PASS matches the PR's HEAD (docs-only PRs skip the PASS).
3. The PR body confirms `code-reviewer` approved, or that every blocker raised is fixed (docs-only PRs skip this).
4. Size is within the guide (about 300 small, 600 medium, warning above 800 source lines), or the PR has the `size-exception` label.
5. The branch, title and every commit subject follow CONTRIBUTING.md and no commit carries a tool trailer.
6. The maintainer said yes to this merge, in the thread or session where the work runs. Approval for one PR does not cover another.

Only if all hold, run `gh pr merge <n> --squash --delete-branch`. Never use `--admin`, `--auto`, a merge commit or rebase merge, or any flag that skips checks.

## After the merge

The squash commit on `main` is the only record; the branch is spent. Clean up the local checkout:

1. `git switch main && git fetch origin main --prune && git merge --ff-only origin/main` (fast-forward only, never commit on `main`).
2. `git branch -D <branch>` (squash merges are never seen as merged by `git branch -d`).
3. Delete the remote branch only if GitHub did not already: `git push origin --delete <branch>`.
4. If a worktree was created: `git worktree remove <path>`, `git worktree prune`, and check `git worktree list` shows no leftovers.
