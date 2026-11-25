# Providers are not named to members

Status: **decided (2026-11-20), implemented (#323 to #326).**

## Decision

Members are no longer told which speech or LLM provider handles their data, and sending meeting text to the configured
LLM no longer needs an escalation. `docs/spec.md` lost its "Data sent to providers" section; the web and the server
notice no longer show provider names; `CLAUDE.md` became the engineering contract. The per-document "May be sent to
the language model" toggle (default off), local hard deletes and keys in the env file stay.

## Consequences

Older specs and records cite the old rule as history. A saved join message that contains `{provider}` shows it
literally. The meeting data still goes to third-party providers: `docs/decisions/hosted-speech.md`.
