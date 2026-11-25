# Meeting detection by microphone holder only

Status: **decided (2026-11-25), implemented (#378, #379, #392); not run on a real call.**
Spec: `docs/specs/meeting-detection.md`.

## Decision

The desktop app notices a call when a known app (Zoom, Teams, Slack, Webex, FaceTime, Discord, Chrome, Edge, Arc,
Safari) holds the microphone, using Core Audio process objects. No Apple Events, browser extension, Accessibility or
Screen Recording permission. It always asks ("Record this meeting?"); there is no auto-record in v1. The "Detect
Meetings" switch is on by default and stored on the Mac.

## Why

It needs no new permission and does not open the microphone. Reading the Meet link from the browser would need
Automation access per browser; that stays an optional backlog ticket (#384).

## Consequences

- Any browser using the microphone counts as a call, so a non-call page can prompt; Safari may be confused with another
  WebKit app.
- The notetaker overlap warning is matched by time, so it warns and never blocks.

## Unverified

Whether muting keeps the microphone held, the 5 s start and 15 s end timings, and the whole flow in the running app.
