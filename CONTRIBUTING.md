# Contributing

## Branches
- `main` is always releasable. Work happens on short-lived branches, one per task, cut from `origin/main`.
- Name: `<type>/<short-kebab-description>`, using the same types as commits: `feat/google-sign-in`,
  `fix/upload-size-limit`, `chore/ci-cache`, `docs/spec-update`. No user or tool prefixes.
- For a ticket, put its code after the type: `<type>/T-<NNN>-<slug>`, e.g. `feat/T-042-ai-provider-settings`
  (NNN is the issue number, at least 3 digits). With no clear ticket use `<type>/<slug>`.
- Delete the branch after merge (local and remote).
- Run `make hooks` once per clone: the pre-push hook refuses pushes to `main` and the pre-commit hook refuses a non-work author or committer email.
  Rewrite your own branch with `git push --force-with-lease`, never `--force`.

## Commits (Conventional Commits)
- Subject: `type(scope): imperative summary`, at most 72 characters, no trailing period. Scope is optional.
- Types: `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `ci`, `perf`, `build`, `revert`.
  Add `!` after the scope for a breaking change.
- Scopes are an area of the code: `server`, `core`, `web`, `auth`, `jobs`, `meetings`, `notetaker`, `speech`, `llm`,
  `deploy`, `desktop` (the macOS app in `desktop/`), `tooling` (hooks, scripts, `.github`, dependency updates).
- Body (optional): wrap at 72, explain why, not what.
- Example:

  ```
  feat(meetings): add token-bucket limit to public API

  Limit is 100 requests per minute per key with bursts to 150.
  Over-limit responses return 429 with a Retry-After header.
  ```
- No AI mentions and no `Co-Authored-By` trailers anywhere (Security & Privacy in `CLAUDE.md`).

## Issues and tickets
- Issues own task status; `docs/specs/` and `docs/decisions/` hold specs and decisions. The full flow, labels and
  merge rules are in `docs/specs/delivery-pipeline.md`.
- Start from an **Intent** issue. A merged spec turns into **Ticket** issues, each one PR.
- Issue titles are `T-<NNN> <title>` where NNN is the issue number (3 digits, zero-padded). The number is only known
  after creation, so rename the issue right after you create it (`gh issue edit <n> --title "T-<NNN> <title>"`).
- Every ticket carries one `status:`, `type:` (`feat`, `bug`, `task`), `priority:` (`high`, `medium`, `low`; default `medium`) and `area:`
  label. Add `needs-review` to an important issue that a human must review and approve before merge.
- Planning: read existing issues, PRs, milestones and docs first; create one issue per planned PR before coding, with
  acceptance, dependencies, a `type:*` label and a milestone set at creation (each new plan gets a new `M<n> <name>`, never an old or finished one; close a milestone with its last issue); confirm with the
  maintainer before creating more than about 10 at once. Docs (spec, decision record, README index) are part of done.
  Never name competitors in issues, PRs, commits or files. Details: the `plan-tickets` skill.
- `Closes #N` in a PR only when it delivers the whole ticket; otherwise `Refs #N`.
- Labels come from `scripts/labels.sh` (safe to re-run); change them there, not by hand in GitHub.

## Pull requests
- One concern per PR; keep it reviewable. Title is `T-<NNN> type(scope): summary` for a ticket, e.g.
  `T-042 feat(desktop): ask to record when a call starts` (NNN is the issue number it closes). With no clear ticket
  the title is `type: description`, e.g. `docs: update deploy status`. The part after the code is the Conventional
  Commit that becomes the squash subject; at most 72 characters in total.
- Description: what changed, how it was tested, and a one-line reason for any new dependency.
- Update the docs the change touches in the same PR (`docs/README.md` says which doc owns what).
- CI runs `guards`, `rust` and `web` on every PR. Commit, title, branch and size rules are not checked in CI: before opening a PR run the scoped checks in
  `CLAUDE.md` (Testing & Verification), `./scripts/check-commits.sh origin/main..HEAD "<PR title>" "<branch>"` and
  `node scripts/pr-size.mjs origin/main`.
- Merge with **squash**; the PR title becomes the commit subject on `main`. Pass the message explicitly so the commit body is short, not the branch's commit list: `gh pr merge <n> --squash --subject "<PR title>" --body "<2-3 line summary>\n\nCloses #<N>"`. The PR body (review record, attribution lines) is not copied. The repository default is the PR title and body, so a merge from the web UI uses the PR body: keep it short there.
