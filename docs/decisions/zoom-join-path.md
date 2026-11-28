# Zoom join path decision

Status: **Meeting SDK bot confirmed by the maintainer (2026-11-27); not built yet, plan in `docs/specs/zoom-notetaker.md`.** Zoom calls can be recorded today with the macOS app ("Meeting app" mode, `docs/specs/desktop-app.md`) or by upload. Several facts below are from general knowledge and must be checked against Zoom's current developer terms before building.

## Options

| Option | Terms and review | Reliability | Audio | Effort |
|---|---|---|---|---|
| Meeting SDK bot on Linux (raw data) | Official route; needs host recording permission; joining meetings outside our own account likely needs app review (rules have changed recently, check) | High | Mixed audio plus a stream per participant | High: native SDK, headless container, virtual audio, forced SDK upgrades |
| Headless browser on the Zoom web client | Automated access likely violates the terms | Low: page changes and bot checks | Mixed only, compressed | Medium to start, high to keep alive |
| Hosted recording-bot service | Vendor handles review | High | Often per speaker | Low |

## Decision

1. **v1: Meeting SDK bot.** Per-participant audio feeds the speaker ribbon directly, so Zoom meetings need no diarization. Start the app review early; it is the longest step.
2. **Rejected (maintainer chose the SDK bot, 2026-11-27): hosted recording-bot service.** It sends audio to a third party, which used to break the rule that audio never leaves the server. That rule ended with hosted speech (2026-11-18, `docs/decisions/hosted-speech.md`), so this reason no longer applies; the other options' trade-offs above are unchanged and the choice is open again.
3. **Rejected for Zoom: headless browser.** Terms risk and fragility. It may still be the only practical route for Google Meet; decide that separately.
4. **Fallback, available now:** upload a Zoom recording through the existing upload feature. Zoom's real-time media streaming for meetings in our own account is worth a look; availability on our plan is unverified.

## Account and app decisions (2026-11-28, ticket #719)

Maintainer answers, plus Zoom's published docs read on 2026-11-28. Only official Zoom developer docs were used; anything not confirmed on those pages is marked unverified.

1. **Account owner:** Tung's personal Zoom developer account. Risk: the app, its credentials and any review are tied to a personal account, so moving to a company account later means creating a new app and redoing the review. Treat the transfer as a known cost; keep credentials in the deploy environment only, never in the repo.
2. **External meetings: not required in v1.** v1 joins meetings the team's own Zoom account hosts. App review: **not started, not needed for v1**. Zoom's docs say apps must go through review to access meetings hosted outside the developer account that created the app, and, since 2026-03-02, apps joining outside their own account must be authorised with an OBF or ZAK token (or RTMS); same-account meetings may, but need not, use an OBF token (source: [Meeting SDK authorization FAQ](https://developers.zoom.us/docs/meeting-sdk/obf-faq/), [OBF transition blog](https://developers.zoom.us/blog/transition-to-obf-token-meetingsdk-apps/)).
   - **Unlisted app for internal use: unverified.** No fetched official page states that an unpublished app may join same-account meetings without review or listing. The FAQ's same-account exemption from OBF suggests it, but this is an inference. Confirm on a real own-account meeting before ticket 4 relies on it.
3. **Raw-data permission (Linux Meeting SDK):**
   - The Linux raw data page lists only technical prerequisites (PulseAudio for audio; in Docker a `zoomus.conf` with `system.audio.type=default`) and says nothing about a separate raw-data licence ([Linux raw data](https://developers.zoom.us/docs/meeting-sdk/linux/default-ui/advanced-features/raw-data/)).
   - The Meetings API has "Get a meeting's join token for local recording": a token that lets a recording bot built on the Meeting SDK join and start local recording without a live host approval. It needs the "Local recording" user setting enabled in the web portal and scope `meeting:read:local_recording_token` (or the `:admin` variant); the token is valid 120 seconds ([Meetings API](https://developers.zoom.us/docs/api/meetings/)). Whether it covers raw recording per meeting and on which plans: **unverified** on the official docs; Linux changelog 5.16.0 mentions a "raw recording token" ([changelog](https://developers.zoom.us/changelog/meeting-sdk/linux/5.16.0/)).
   - **Host approval per meeting: unverified in official docs.** Zoom developer forum threads (not official docs, treat as hints) say a bot without the token must be host/co-host or have "allow to record local files", can call `RequestLocalRecordingPrivilege` which prompts the host, and that no separate raw-data entitlement is needed on Business, Education and Enterprise plans. Plan on a host prompt or the join token; verify in a spike.
   - **App review for raw data: unverified.** Review is documented only for joining meetings outside the developer account (item 2).
   - **Policy risk, unverified:** one search snippet of the Meeting SDK page said the SDK is for human use and does not support bots or AI notetakers, pointing to RTMS; the page fetched directly did not contain that text. Check this before building: if true, RTMS (real-time media streams) may be the supported route, and the spec lists it as out of scope.

**App type: Meeting SDK** (Linux, raw data), as decided above. It is the only Zoom type that joins as a participant and gives per-participant raw audio, which feeds the speaker ribbon without diarization. Alternatives: RTMS (supported for notetakers and needs no join, but a different review and delivery model; revisit if the policy risk above is confirmed), and a plain OAuth or Server-to-Server app, which cannot join a call.

