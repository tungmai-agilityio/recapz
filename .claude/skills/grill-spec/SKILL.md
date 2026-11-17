---
name: grill-spec
description: Turn an `idea` issue into an agreed spec file by grilling the maintainer, then open the docs-only spec PR. Use when the maintainer names an idea issue to plan, or says "grill" or "write the spec". Stage 2 of docs/specs/delivery-pipeline.md.
---

# Grill an idea into a spec

Input: an idea issue number. Output: `docs/specs/<feature>.md` in a docs-only PR that says `Refs #<idea>`.
The maintainer's confirmation at the end of the grilling is the approval. Merge the spec PR yourself once CI is
green on its HEAD, then stop; never create tickets here (`plan-tickets` does that after the merge). If the spec turns
out to be critical under "Merging and escalation" in `CLAUDE.md` (for example it bends a hard rule), leave the PR
for the maintainer.

Use `gh` as the account that has access to this repository. If several accounts are logged in, set `GH_TOKEN` for the command; never run `gh auth switch`.

## Steps

1. **Read the idea.** `gh issue view <n>`. It must be `kind:idea`; otherwise stop. Move it to planning:
   `gh issue edit <n> --remove-label status:backlog --add-label status:planning`.
2. **Read what exists, so you never ask what the repo answers.** `docs/spec.md`, `CLAUDE.md` (clean-room and hard
   rules), the specs and decisions in `docs/specs/` and `docs/decisions/` that touch the idea, and for web work
   `docs/ui-parity.md`. Use `explore-code` for broad code searches.
3. **Grill.** Load `mattpocock-skills:grilling` with the Skill tool and follow it: rounds of numbered questions,
   each with a recommended answer, asked through the interactive question UI; facts are your job, decisions are
   the maintainer's. If that skill is not installed, do the same loop yourself.
   Cover at least: the problem and who has it, the decisions, what is out of scope, acceptance, privacy and
   data-deletion impact, whether any language-model provider receives new data, and how the work splits into
   one-PR tickets.
4. **Stop when the frontier is empty** and the maintainer confirms a shared understanding. Do not write the spec
   before that.
5. **Write the spec** from `docs/specs/_template.md` to `docs/specs/<feature>.md`.
   - Tickets: one PR each, split anything over about 600 source lines, give `touches`, `blocked by`, size and
     testing scope. Name each ticket `type(scope): summary` with a `type:` and `area:` from
     `docs/specs/delivery-pipeline.md`.
   - Text outside `.claude/` must follow the clean-room rules: no AI mentions, no other product names, no copied
     wording from other products.
6. **Open the PR** with the `pull-request` skill: branch `docs/<feature>-spec`, title
   `docs(specs): add <feature> spec`, body ends with `Refs #<idea>` (not `Closes`). Docs-only: skip
   `test-runner` and `code-reviewer`, write "docs only, no tests run" under Testing. Wait for CI, then merge it as the
   `pull-request` skill says. Leave the idea at `status:planning`; `plan-tickets` closes it.

## Stop and escalate

If the idea contradicts `CLAUDE.md` or an existing spec, or the maintainer cannot answer a blocking question:
set `status:needs-human` on the idea with a comment saying why, and stop.
