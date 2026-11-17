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

S2 stays unticked: the list is still the plain first version, and the Send notetaker form is an inline box with no prototype screenshot yet. A per-row Delete with confirm is done, shown on
every row because the list API has no owner info (a refusal appears in the dialog). Hide it for non-owners
once the list returns the owner.

## S3 Meeting report
- [ ] Header: back link, title, date, duration, platform, faces; Share, Regenerate, Delete with confirm
      (Delete with confirm is done, in a "Meeting actions" menu; Share and Regenerate wait on their own tasks,
      the date, duration and faces on the header redesign). Rename is also in that menu, with an inline title
      editor; the prototype header has neither, so the row stays unticked until the maintainer confirms the
      design, and no screenshot is attached yet (not checked by eye))
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
- [ ] Live Ask panel ("Only you see these"): context line (last 60 s, or a clicked transcript line, with "use last 60 s"),
      optional extra words, Ask now button and `A` hotkey; answer cards newest first with asker, quote, time taken, answer,
      and Document or Meeting sources with page or time; "Passages only" card for documents not sent to the language model;
      waiting state (`docs/live-ask-plan.md`)

## S9 My documents (blocked: documents API, `docs/live-ask-plan.md` row 4)
- [ ] Drop zone: PDF with selectable text, Markdown, TXT, up to 20 MB each
- [ ] Table: name, type, size, added date; status Indexed, Indexing, or No text found (scanned); "Send to language model"
      switch per row (`role=switch`, off by default, disabled when no text); Delete with inline confirm
- [ ] Aside: storage meter (of 500 MB) and what the switch means when off and on

## S7 Members (blocked: members API)
- [ ] Invite form (email, role) with validation errors, disabled for non-admins
- [ ] Members table: face, name, email, role select, meetings, pending badge, Remove or Cancel invite with inline confirm
- [ ] Default sharing and Who can sign in radio groups

## S8 Recording & privacy (blocked: settings API)
- [x] Calendar row: Google Calendar tile with Connected, Reconnect needed or Not connected, Connect or Reconnect, Disconnect with confirm, and the `?calendar=` outcome message (both skins, mobile checked by eye)
- [ ] Notetaker sign-in status on this screen: no API exposes it yet (a signed-out account only shows as a failed meeting's error), so there is nothing to show
- [ ] Join rule radios, notetaker name, join message
- [ ] Retention selects (audio, transcripts); "Where data lives" read-only text
- [ ] Delete everything one participant said, with confirm; Save changes; saved toast
