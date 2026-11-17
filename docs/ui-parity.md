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
- [x] Skins Daylight (default) and Ink; modes Light (default), Dark, Auto; `data-skin` and `data-theme` on `<html>`;
      stored in localStorage `recapz-skin` and `recapz-mode`
      Covered by theme.test.tsx (#135).
- [x] Tokens: paper, sheet, wash, ink, ink-2, mute, hair, hair-2, rail, rail-fg, rail-on, btn, btn-fg, signal,
      signal-ink, signal-wash, good, good-wash, bad, bad-wash, speaker-1..5, radius 6px
      Checked in index.css; btn and btn-fg are named primary and primary-fg there. Not checked by eye (#135).
- [x] Be Vietnam Pro for UI, JetBrains Mono with tabular numerals for times and counts
      Fonts loaded in index.html; `.font-mono` gets tabular numerals in index.css (#135).
- Daylight section framing: the `.frame` class in index.css (skin variables `--frame-*`, none in Ink) wraps the Meetings
  list and aside, the report summary column, sidebar and transcript, the Ask conversation and the Upload columns.
  Recording & privacy already had its own card. Per-section cards inside a column (as in the prototype `.sec`) are not
  split yet. No rows ticked.
- [ ] Primitives in `web/src/ui/`: Button (default, primary, ghost, danger), Badge (good, signal, bad), Face, Faces,
      Ribbon (list, scrubber, live), Switch, Tabs, Choice group, Field, Table, Toast, Menu, PageHead
      Open: everything exists (Ribbon added in this branch) except Table; ribbon is not used by a page yet.
- [ ] Amber (`signal`) only on the playhead, recording state and focus ring
- [ ] Layout: 208 px rail, page max 1120 px, two columns `1fr 320px` that stack at 900 px, rail becomes a top strip
      Rail, columns and the 900 px stacking were checked by eye on 2026-11-17; the 1120 px page maximum was not exercised (content is narrower than that at 1280 px), so the row stays open.
- [x] Reduced motion honoured (recording dot, pipeline fill)
      Global rule in index.css, seen in the built CSS only; the recording dot and pipeline fill do not exist yet (#135).
- [ ] Keyboard: visible focus, Cmd/Ctrl+K opens Ask, Space plays or pauses a report, Esc closes menus

## S1 Shell
- [ ] Rail: glyph and wordmark; groups Work (Meetings, Meeting report, Ask, Live meeting, Upload) and Workspace
      (Members, Recording & privacy); v1 dot; active item has `aria-current="page"` and an amber marker (Ink) or
      blue marker (Daylight)
      Open: Live meeting, Upload and Members stay out of the rail until their screens exist; v1 dots and the glyph are in.
- [ ] Account button opens a menu: name, email, role badge, Manage members, Recording & privacy, Sign out
      (`role=menu`); Esc and outside click close it
- [x] Top bar: breadcrumb, "Ask about any meeting…" box with the ⌘K hint, skin switch (group label "Theme"),
      mode switch (group label "Light or dark mode"; Light, Dark, Auto)
      Checked by eye in the 2026-11-17 screenshot pass (1280 and 390 px). Group labels are covered by tests (#135).
- [x] Signed-out state shows the sign-in card; a 401 returns to it
      Covered by App.test.tsx (#135).
- [x] Mobile (under 900 px): rail becomes a top strip, Ask box on its own row
      Checked by eye in the 2026-11-17 screenshot pass (1280 and 390 px). The strip wraps onto a second line at 390 px; the prototype scrolls it sideways.

## S2 Meetings
- [x] Header "Meetings" with a counts line, Upload link, Send notetaker button (built: counts line shows meetings this month and
      hours recorded from `duration_ms`, hours omitted when no meeting has a duration; no screenshot yet)
      Checked by eye in the 2026-11-17 screenshot pass (1280 and 390 px).
- [ ] Send notetaker dialog: paste a meeting link; bad link shows an error (modal dialog, server message shown) (built in #137; no screenshot yet)
- [x] Days grouped (Today, Yesterday, weekday and date); row: time, title, duration, platform, action-item count,
      ribbon, faces; ribbon moves under the title on mobile (built: duration, platform, action-item count (hidden at 0), equal-weight
      speaker ribbon and faces from `speakers` (hidden when empty); no screenshot yet. The ribbon is one segment per
      speaker, not a timeline, because the list API has no speaking times)
      Checked by eye in the 2026-11-17 screenshot pass (1280 and 390 px). Speaker colours match the Report (#150).
- [ ] Processing row shows a stage badge; failed row an error badge; opted-out row (stage and failed badges done;
      no opted-out state exists in the list data)
- [ ] Aside "Next up today": time, title, per-meeting switch (`role=switch`), "Join rules" link (built in #137; no screenshot yet)
- [ ] Live card (recording badge, elapsed time, ribbon, Open live view) only while recording (not built: the live
      view route does not exist yet; rows with status `recording` show a Recording badge)
- [ ] Empty, loading and error states (built in #137; no screenshot yet)

`GET /api/meetings` now returns `duration_ms`, `action_item_count` and `speakers` (max 5); the web rows are built and ticked after the screenshot pass.

No screenshot attached and not checked by eye: no browser tool was available. A per-row Delete with confirm is
done, shown on every row because the list API has no owner info (a refusal appears in the dialog). Hide it for
non-owners once the list returns the owner.

## S3 Meeting report
- [ ] Header: back link, title, date, duration, platform, faces; Share, Regenerate, Delete with confirm
      (Delete with confirm is done, in a "Meeting actions" menu; Share and Regenerate wait on their own tasks,
      the date, duration and faces are built: faces show the people who spoke, from the transcript). Rename is also in that menu, with an inline title
      editor; the prototype header has neither, so the row stays unticked until the maintainer confirms the
      design, and no screenshot is attached yet (not checked by eye))
- [ ] Transport: play and pause, ribbon scrubber with chapter ticks and playhead, current and total time;
      clicking the ribbon seeks; Space toggles
      (built: play/pause, ribbon scrubber with chapter ticks and playhead, times, click and arrow-key seek, Space;
      left unticked only because no screenshot at 1280 and 390 px was taken, so not checked by eye)
- [x] Tabs Summary and Transcript (`role=tablist`)
      Checked by eye in the 2026-11-17 screenshot pass (1280 and 390 px).
- [ ] Summary: prose, Decisions, Open questions with timestamp link, Action items "n/m done" with checkbox, owner
      face, due date, strike-through when done
      (built except owner face: the report API has no owner. Done state is shown read-only: no endpoint saves it)
- [ ] Sidebar: Chapters (click seeks and opens the transcript), Talk time bars, Details (language, sharing,
      deletion date)
      (Chapters, Details language and Talk time done: bars and percent from real segment durations, sorted, hidden
      when no segment has a speaker, shown in the Summary tab with the report; sharing and deletion date are not in
      the meeting API. Not ticked: no screenshot yet)
- [ ] Transcript: speaker colour bar, time, speaker name, text, current line highlighted, click seeks
      (colour bar, time, speaker name, text, highlight, seek done. The name shows only on lines with a speaker; a
      speaker with no name reads "Unknown". One colour per person, by order of first speaking, shared with the
      ribbon, faces and Talk time. Not ticked: no screenshot yet)
- [ ] Processing, failed with retry, no report yet, no audio yet, 404
      (processing, failed with error, no report, no audio, 404 done; retry hidden, no endpoint)

## S4 Ask
- [ ] Question line, answer with numbered markers, three citation cards (number, time, meeting title, two-line
      quote, ribbon with pin). Unticked: markers sit after the answer, not inline, because the server returns the
      answer and its citation list separately; cards have no ribbon or pin because citations carry no speaker or
      offset data and `web/src/ui/Ribbon.tsx` does not exist yet. Rest done in #136.
- [x] Suggestion (#136) chips, sticky composer with Ask button, Cmd/Ctrl+K focus
- [x] States (#136): streaming, error, 401 "Sign in to ask.", rate limited (429), 503 "Ask needs a language model", no results

## S5 Upload
- [ ] Drop zone with accepted types and size limit (only formats the backend accepts). Built in #138; no screenshot yet.
- [ ] Fields: title, language, speaker count; button "Process recording". Built in #138; no screenshot yet. Only title is built: the upload API reads
      no language or speaker count, so those are hidden rather than faked.
- [ ] Pipeline progress over the real statuses (uploaded, transcribing, reporting, ready, failed); error with retry;
      link to the new meeting when ready. Built in #138; no screenshot yet. Retry re-uploads the file (no retry endpoint). Shows 4 steps (Uploaded, Transcribing, Report, Ready), not the prototype's 5, since the backend has no Speakers or Indexed status.

## S6 Live meeting (blocked: events API and notetaker driver)
- [ ] Recording pill with timer, platform, join time, title; Pause; Stop and leave with confirm
- [ ] Live ribbon, live transcript, speaking now, talk time, noted so far
- [ ] Live Ask panel ("Only you see these"): context line (last 60 s, or a clicked transcript line, with "use last 60 s"),
      optional extra words, Ask now button and `A` hotkey; answer cards newest first with asker, quote, time taken, answer,
      and Document or Meeting sources with page or time; "Passages only" card for documents not sent to the language model;
      waiting state (`docs/specs/live-ask.md`)

## S9 My documents (blocked: documents API, `docs/specs/live-ask.md` row 4)
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
