# Local self-signed code signing for the macOS app

Status: **decided (2026-11-23), implemented (#359).**
Spec: `docs/specs/desktop-app.md`, "Local builds, self-signed"; runbook: `docs/deploy.md`, "macOS app".

## Decision

The app is built on the maintainer's Mac and signed with a self-signed "Recapz Local" certificate created by
`scripts/desktop-cert.sh`. There is no Apple Developer account, no notarization and no macOS CI. Members right-click →
Open once.

## Why

Cost: the account is $99 a year and macOS runners are billed 10x on the free plan. A stable certificate keeps the
microphone and audio-capture grants across builds (*unverified*: recorded as an expectation only).

## Consequences

Gatekeeper warns on first open. Replacing the certificate makes macOS treat the app as new and ask for grants again.
Buy a Developer ID when members find the first open painful or the team grows.
