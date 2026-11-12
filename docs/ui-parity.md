# UI parity checklist

The Recapz prototype is the visual spec (guide §10: Daylight and Ink skins, light and dark). This file is the
definition of done for every web PR.

## How to use it
- A web PR ticks the rows of the screen it touches (`[x]` plus the PR number) and attaches a screenshot next to the
  prototype state. An unticked row needs a written reason beside it.
- Screenshots come from a real browser at 1280 px and 390 px wide. Shell, token and primitive PRs and the last PR of
  each screen also cover Daylight and Ink, light and dark.
- A server PR that adds an endpoint for a screen names the row it unblocks.
- Never ship prototype sample data (names, workspace, the "Prototype · sample data" badge). Test data is neutral.
- Hide what has no backend; never fake it (no random ribbons, no inert buttons).
- Screens marked "after v1" in the prototype (Catch-up, Talk stats, Inbox digest, Follow-ups) are not built and not
  shown in the rail. Connections and "Send to tracker" are skipped until their backends exist.

## Global
- [ ] Skins Daylight (default) and Ink; modes Light (default), Dark, Auto; `data-skin` and `data-theme` on `<html>`;
      stored in localStorage `recapz-skin` and `recapz-mode`
- [ ] Tokens: paper, sheet, wash, ink, ink-2, mute, hair, hair-2, rail, rail-fg, rail-on, btn, btn-fg, signal,
      signal-ink, signal-wash, good, good-wash, bad, bad-wash, speaker-1..5, radius 6px
- [ ] Be Vietnam Pro for UI, JetBrains Mono with tabular numerals for times and counts
- [ ] Primitives in `web/src/ui/`: Button (default, primary, ghost, danger), Badge (good, signal, bad), Face, Faces,
      Ribbon (list, scrubber, live), Switch, Tabs, Choice group, Field, Table, Toast, Menu, PageHead
- [ ] Amber (`signal`) only on the playhead, recording state and focus ring
- [ ] Layout: 208 px rail, page max 1120 px, two columns `1fr 320px` that stack at 900 px, rail becomes a top strip
- [ ] Reduced motion honoured (recording dot, pipeline fill)
- [ ] Keyboard: visible focus, Cmd/Ctrl+K opens Ask, Space plays or pauses a report, Esc closes menus

## S1 Shell
- [ ] Rail: glyph and wordmark; groups Work (Meetings, Meeting report, Ask, Live meeting, Upload) and Workspace
      (Members, Recording & privacy); v1 dot; active item has `aria-current="page"` and an amber marker (Ink) or
      blue marker (Daylight)
- [ ] Account button opens a menu: name, email, role badge, Manage members, Recording & privacy, Sign out
      (`role=menu`); Esc and outside click close it
- [ ] Top bar: breadcrumb, "Ask about any meeting…" box with the ⌘K hint, skin switch (group label "Theme"),
      mode switch (group label "Light or dark mode"; Light, Dark, Auto)
- [ ] Signed-out state shows the sign-in card; a 401 returns to it
- [ ] Mobile (under 900 px): rail becomes a top strip, Ask box on its own row

## S2 Meetings
- [ ] Header "Meetings" with a counts line, Upload link, Send notetaker button
- [ ] Send notetaker dialog: paste a meeting link; bad link shows an error
- [ ] Days grouped (Today, Yesterday, weekday and date); row: time, title, duration, platform, action-item count,
      ribbon, faces; ribbon moves under the title on mobile
- [ ] Processing row shows a stage badge; failed row an error badge; opted-out row
- [ ] Aside "Next up today": time, title, per-meeting switch (`role=switch`), "Join rules" link
- [ ] Live card (recording badge, elapsed time, ribbon, Open live view) only while recording
- [ ] Empty, loading and error states

## S3 Meeting report
- [ ] Header: back link, title, date, duration, platform, faces; Share, Regenerate, Delete with confirm
- [ ] Transport: play and pause, ribbon scrubber with chapter ticks and playhead, current and total time;
      clicking the ribbon seeks; Space toggles
- [ ] Tabs Summary and Transcript (`role=tablist`)
- [ ] Summary: prose, Decisions, Open questions with timestamp link, Action items "n/m done" with checkbox, owner
      face, due date, strike-through when done
- [ ] Sidebar: Chapters (click seeks and opens the transcript), Talk time bars, Details (language, sharing,
      deletion date)
- [ ] Transcript: speaker colour bar, time, speaker name, text, current line highlighted, click seeks
- [ ] Processing, failed with retry, no report yet, no audio yet, 404

## S4 Ask
- [ ] Question line, answer with numbered markers, three citation cards (number, time, meeting title, two-line
      quote, ribbon with pin)
- [ ] Suggestion chips, sticky composer with Ask button, Cmd/Ctrl+K focus
- [ ] States: streaming, error, 401 "Sign in to ask.", rate limited (429), 503 "Ask needs a language model", no results

## S5 Upload
- [ ] Drop zone with accepted types and size limit (only formats the backend accepts)
- [ ] Fields: title, language, speaker count; button "Process recording"
- [ ] Pipeline progress over the real statuses (uploaded, transcribing, reporting, ready, failed); error with retry;
      link to the new meeting when ready

## S6 Live meeting (blocked: events API and notetaker driver)
- [ ] Recording pill with timer, platform, join time, title; Pause; Stop and leave with confirm
- [ ] Live ribbon, live transcript, speaking now, talk time, noted so far

## S7 Members (blocked: members API)
- [ ] Invite form (email, role) with validation errors, disabled for non-admins
- [ ] Members table: face, name, email, role select, meetings, pending badge, Remove or Cancel invite with inline confirm
- [ ] Default sharing and Who can sign in radio groups

## S8 Recording & privacy (blocked: settings API)
- [ ] Join rule radios, notetaker name, join message
- [ ] Retention selects (audio, transcripts); "Where data lives" read-only text
- [ ] Delete everything one participant said, with confirm; Save changes; saved toast
