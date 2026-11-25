# Zoom join path decision

Status: **proposed, never confirmed; no Zoom join path is built (2026-11-25).** Zoom calls can be recorded today with the macOS app ("Meeting app" mode, `docs/specs/desktop-app.md`) or by upload. Several facts below are from general knowledge and must be checked against Zoom's current developer terms before building.

## Options

| Option | Terms and review | Reliability | Audio | Effort |
|---|---|---|---|---|
| Meeting SDK bot on Linux (raw data) | Official route; needs host recording permission; joining meetings outside our own account likely needs app review (rules have changed recently, check) | High | Mixed audio plus a stream per participant | High: native SDK, headless container, virtual audio, forced SDK upgrades |
| Headless browser on the Zoom web client | Automated access likely violates the terms | Low: page changes and bot checks | Mixed only, compressed | Medium to start, high to keep alive |
| Hosted recording-bot service | Vendor handles review | High | Often per speaker | Low |

## Decision

1. **v1: Meeting SDK bot.** Per-participant audio feeds the speaker ribbon directly, so Zoom meetings need no diarization. Start the app review early; it is the longest step.
2. **Rejected: hosted recording-bot service.** It sends audio to a third party, which used to break the rule that audio never leaves the server. That rule ended with hosted speech (2026-11-18, `docs/decisions/hosted-speech.md`), so this reason no longer applies; the other options' trade-offs above are unchanged and the choice is open again.
3. **Rejected for Zoom: headless browser.** Terms risk and fragility. It may still be the only practical route for Google Meet; decide that separately.
4. **Fallback, available now:** upload a Zoom recording through the existing upload feature. Zoom's real-time media streaming for meetings in our own account is worth a look; availability on our plan is unverified.

## Open questions

- Is the team's Zoom account allowed to run an unlisted app for internal use, or is listing required?
- Which meetings must be joined: only ones the team hosts, or external ones too?
- Who owns the Zoom developer account?
