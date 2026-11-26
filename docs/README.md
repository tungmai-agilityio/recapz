# Docs index

Which document owns what. When two disagree, the one named here wins; fix the other.

| Need | Source of truth |
|---|---|
| What Recapz is, vocabulary, rules, API guide | `docs/spec.md` |
| Where code lives, data flow | `docs/architecture.md` |
| Why a choice was made | `docs/decisions/` (index in its `README.md`) |
| What a feature does and its tickets | `docs/specs/<feature>.md` (Status line says what merged) |
| Run it locally, desktop app against it | `docs/running-locally.md` |
| Host, release, backups, macOS app release | `docs/deploy.md` |
| Screens against the prototype | `docs/ui-parity.md`; canonical UI reference: https://claude.ai/artifact/KxQaWLV4L9kusPFz1eonzH |
| How ideas become releases, merge rules | `docs/specs/delivery-pipeline.md` |
| Branches, commits, PRs | `CONTRIBUTING.md` |
| Engineering contract | `CLAUDE.md` |
| Open work and checks not yet run | GitHub issues; what each feature has not verified is in its spec's Status line |
| Coding-session agents and skills | `.claude/agents/`, `.claude/skills/` |

## Keeping docs current

A PR that changes behaviour, config, a command, a route or a decision updates the doc above in the same PR (the PR
template has a line for it). Write what was not run as *unverified*; do not state it as fact. Status lines in specs
change when the last ticket merges.
