# UI parity checklist

The Recapz prototype (https://claude.ai/artifact/KxQaWLV4L9kusPFz1eonzH) is the canonical UI reference and the visual spec (guide §10: Daylight and Ink skins, light and dark). This file is the
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
- [x] Primitives in `web/src/ui/`: Button (default, primary, ghost, danger), Badge (good, signal, bad), Face, Faces,
      Ribbon (list, scrubber, live), Switch, Tabs, Choice group, Field, Table, Toast, Menu, PageHead
      All thirteen exist (Table is used by Documents, Faces by Next up, Ribbon by the Report) and show on the
      Meetings, Report, Upload and Documents screens checked by eye on 2026-11-20 (#332, #327, #328, #331);
      `ui.test.tsx` and `display.test.tsx` cover them.
- [ ] Amber (`signal`) only on the playhead, recording state and focus ring
      Checked in code and in a browser on 2026-11-20 (#332): the playhead, the Recording badge and the 2 px focus ring
      are amber (`rgb(227, 154, 45)`). Not ticked: amber is also the stage badge for Uploaded, Joining, Labelling
      speakers, Indexing, Transcribing and Writing report (`meeting-days.ts`, Documents), the wash behind the
      current transcript line (Report) and the selected line in the live view, and the amber bar of the logo glyph in the rail.
      The prototype uses an amber badge for "Transcribing" too, so the rule may need to say so.
- [x] Layout: 208 px rail, page max 1120 px, two columns `1fr 320px` (`ui/Columns`) that stack when the page area is
      under 56rem, rail becomes a top strip under 900 px
      Only the Shell (rail, top bar) reacts to the window. Everything inside `<main>` reacts to its container: `<main>`
      is `@container`, and a column or card whose rows need to change shape is its own `@container`; use `@max-*:` and
      `@*:` variants, never `sm:`/`md:`/`lg:` or `max-[Npx]:`. `src/layout.test.ts` fails on a window breakpoint outside
      Shell. Check a screen at 1000 px as well as 1280 and 390: that is where the rail plus a side column leaves the
      least room. Columns and stacking were checked at 920, 1000, 1280 and 390 px on 2026-11-19, and on 2026-11-20
      (#332) `<main>` measured exactly 1120 px in a 1920 px window.
- [x] Reduced motion honoured (recording dot, pipeline fill)
      Global rule in index.css, seen in the built CSS only; the recording dot and pipeline fill do not exist yet (#135).
- [x] Keyboard: visible focus, Cmd/Ctrl+K opens Ask, Space plays or pauses a report, Esc closes menus
      Checked in a real browser on 2026-11-20 (#332): Tab shows a 2 px amber outline, Meta+K and Control+K both go to
      `/ask`, Esc closes the Account menu, Space toggles playback (#327). Esc on other menus uses the same `Menu`.

## S1 Shell
- [ ] Rail: glyph and wordmark; groups Work (Meetings, Meeting report, Ask, Live meeting, Upload) and Workspace
      (Members, Recording & privacy); v1 dot; active item has `aria-current="page"` and an amber marker (Ink) or
      blue marker (Daylight)
      Checked by eye on 2026-11-20 (#332) at 1280 and 390 px, Daylight and Ink: the blue marker (Daylight) and amber
      marker (Ink) show on the active item, groups and v1 dots match. Not ticked: the rail has no "Meeting report"
      item (the prototype lists one) and has "My documents", which the prototype's Work group also lists; Live meeting
      was not seen because no call was recording; `aria-current="page"` is on both the wordmark link and Meetings.
      Open: Members stays out of the rail until its screen exists. Live meeting appears only while a call is being recorded, one item per recording call, named by its title when there are several (the rail asks `GET /api/meetings` every 15 s); v1 dots and the glyph are in.
- [ ] Account button opens a menu: name, email, role badge, Manage members, Recording & privacy, Sign out
      (`role=menu`); Esc and outside click close it
      Checked in a real browser on 2026-11-20 (#332): the menu shows the email, an Admin badge, Recording & privacy and
      Sign out; Esc and a click outside close it. Not ticked: no separate name line, no Manage members (Members is
      not built), and the email breaks mid-word ("maidangtung95@g / mail.com") in the 208 px menu.
- [x] Top bar: breadcrumb, "Ask about any meeting…" box with the ⌘K hint, skin switch (group label "Theme"),
      mode switch (group label "Light or dark mode"; Light, Dark, Auto)
      Checked by eye in the 2026-11-17 screenshot pass (1280 and 390 px). Group labels are covered by tests (#135).
- [x] Signed-out state shows the sign-in card; a 401 returns to it
      Covered by App.test.tsx (#135).
- [x] Mobile (under 900 px): rail becomes a top strip, Ask box on its own row
      Checked by eye in the 2026-11-17 screenshot pass (1280 and 390 px). The strip scrolls sideways like the prototype and keeps the current screen's link in view; the logo and account button stay fixed so the menu is not clipped. Rechecked on 2026-11-26 at 390 px, Daylight light.

## S2 Meetings
- [x] Header "Meetings" with a counts line, Upload link, Send notetaker button (built: counts line shows meetings this month and
      hours recorded from `duration_ms`, hours omitted when no meeting has a duration)
      Checked by eye in the 2026-11-17 screenshot pass (1280 and 390 px) and again on 2026-11-20 (#331) against the
      prototype, Daylight light, Ink dark. The prototype adds "all audio stays on your server" to the counts line and
      has no "Record from this browser" button; the app has both buttons the prototype lacks and omits that clause.
- [x] Send notetaker dialog: paste a meeting link; bad link shows an error (modal dialog, server message shown) (built in #137)
      Checked in a real browser on 2026-11-20 (#331): the dialog opens over the list, a text that is not a URL is
      stopped by the browser's own check, and `https://example.com/not-a-call` shows the server's "not a Meet or Zoom
      link" inside the dialog. The prototype has no dialog (its button only shows a toast), so there is no state to
      compare.
- [x] Days grouped (Today, Yesterday, weekday and date); row: time, title, duration, platform, action-item count,
      ribbon, faces; ribbon moves under the title on mobile (built: duration, platform, action-item count (hidden at 0), equal-weight
      speaker ribbon and faces from `speakers` (hidden when empty); no screenshot yet. The ribbon is one segment per
      speaker, not a timeline, because the list API has no speaking times)
      Checked by eye in the 2026-11-17 screenshot pass (1280 and 390 px). Speaker colours match the Report (#150).
      Rechecked on 2026-11-20 (#331): grouping, time, title, duration, platform and the stacked rows at 390 px match;
      the test meetings have no speakers or action items, so the ribbon, faces and count were not seen. The prototype
      shows 24-hour times and "Monday 9 Nov"; the app shows "04:22 PM" and "Tuesday, November 17", and a Delete
      button on every row that the prototype does not have. At 390 px the app puts the time under the title.
- [ ] Processing row shows a stage badge; failed row an error badge; opted-out row (stage and failed badges done;
      no opted-out state exists in the list data)
      Failed badge checked by eye on 2026-11-20 (#331) at 1280 and 390 px, Daylight light and Ink dark (an upload
      with an invalid speech key). Not ticked: the processing badge was not captured (the job failed within seconds),
      the prototype's "Transcribing · 64%" has no percent in the app, and there is no opted-out state.
- [ ] Aside "Next up today": time, title, per-meeting switch (`role=switch`), "Join rules" link (built in #137). The time and "Open link" no longer break across lines; checked in Chromium at 1280, 1100 and 390 px, Daylight light only, with screenshots on the fix PR. Left unticked: on 2026-11-20 (#331) no calendar was connected, so the card showed only its empty text ("Nothing else today. Connect Google Calendar on Recording & privacy.") at 1280 and 390 px; rows and switches were not seen against the prototype's
- [ ] Live card (recording badge, elapsed time, ribbon, Open live view) only while recording
      Open: built without the ribbon (the live stream has no speaker labels): Recording badge, title, platform, time
      since the call was admitted (counts every second), and an "Open live view" link; one card per recording call
      above the list. Checked in jsdom only: no call was recording on 2026-11-20 (#331), so the card was not seen. The prototype shows its ribbon and "Open live view" in the aside; the app puts the card above the list. Rows with status `recording`
      also keep their Recording badge, and the rail links to the live view.
- [ ] Empty, loading and error states (built in #137)
      Not ticked: on 2026-11-20 (#331) only a blank page while loading was seen; the list had meetings and the server
      was up, so the empty and error states were not shown.

`GET /api/meetings` now returns `duration_ms`, `action_item_count` and `speakers` (max 5); the web rows are built and ticked after the screenshot pass.

A per-row Delete with confirm is
done, shown on every row because the list API has no owner info (a refusal appears in the dialog). Hide it for
non-owners once the list returns the owner.

## S3 Meeting report
- [ ] Header: back link, title, date, duration, platform, faces; Share, Regenerate, Delete with confirm
      (Delete with confirm is done, in a "Meeting actions" menu; Regenerate is in that menu with a confirm (owner or admin, ready meetings only); Share waits on its own task,
      the date, duration and faces are built: faces show the people who spoke, from the transcript). Rename is also in that menu, with an inline title
      editor; the prototype header has neither, so the row stays unticked until the maintainer confirms the
      design. Checked by eye on 2026-11-20 (#327) at 1280 and 390 px: title, date, duration and platform match.
      Not checked: faces (the test meeting has no speakers), and the prototype's Share, Regenerate and Delete
      icons, which the app replaces with the "Meeting actions" menu.)
- [x] Transport: play and pause, ribbon scrubber with chapter ticks and playhead, current and total time;
      clicking the ribbon seeks; Space toggles
      Checked in a real browser on 2026-11-20 (#327) against a 5:00 recording at 1280 and 390 px: Play and Pause
      switch the button and move the time; a click at the middle of the ribbon seeks to 2:30; Left and Right move
      5 s; Space toggles with the focus off the controls. Home and End do nothing. The ribbon was grey here because
      that meeting has no speaker data, so speaker colours on the ribbon are not checked.
- [x] Tabs Summary and Transcript (`role=tablist`)
      Checked by eye in the 2026-11-17 screenshot pass (1280 and 390 px).
- [ ] Summary: prose, Decisions, Open questions with timestamp link, Action items "n/m done" with checkbox, owner
      face, due date, strike-through when done
      (built: checkbox, owner select with face, due date input and "n/m done" save through PATCH /api/action-items/{id} (#410); no prototype screenshot compared yet)
      Checked by eye on 2026-11-20 (#327): prose, Decisions and Open questions show real data at 1280 and 390 px.
      Not ticked: the prototype uses small uppercase muted section labels, a blue square bullet with a hairline
      between Decisions, and an amber dot with a mono timestamp link on Open questions; the app uses larger
      sentence-case headings and plain disc bullets, and the sample report has no action items or timestamp link.
- [ ] Sidebar: Chapters (click seeks and opens the transcript), Talk time bars, Details (language, sharing,
      deletion date)
      (Chapters, Details language and Talk time done: bars and percent from real segment durations, sorted, hidden
      when no segment has a speaker, shown in the Summary tab with the report; Shared with is built (a select for the owner or an admin, read-only text for others; no prototype screenshot, so
      not ticked); the deletion date is not in
      the meeting API. Not ticked: Chapters and Details language checked by eye on 2026-11-20 (#327) at 1280 and
      390 px, but Talk time could not be seen (the test meeting has no speakers), the prototype uses uppercase
      muted labels for Chapters, Talk time and Details, and its Details also show Shared with and the deletion date;
      admins also see a Cost line (speech, language model) under Details: covered by a jsdom test only, not compared to the prototype or seen in a browser, so unverified)
- [ ] Transcript: speaker colour bar, time, speaker name, text, current line highlighted, click seeks
      (colour bar, time, speaker name, text, highlight, seek done. The name shows only on lines with a speaker; a
      speaker with no name reads "Unknown". One colour per person, by order of first speaking, shared with the
      ribbon, faces and Talk time. Not ticked: line text, time and seek checked by eye on 2026-11-20 (#327), but the
      test meeting has no speakers, so the colour bar, speaker name and Unknown label were not seen)
- [ ] Processing, failed with retry, no report yet, no audio yet, 404
      (processing, failed with error, no report, no audio, 404 done; retry is a button for the owner or an admin)
      Not ticked: only 404 was checked by eye (2026-11-20, #327); the other states have tests in Report.test.tsx
      but no meeting in that state to screenshot.

## S4 Ask
- [x] Question line, answer with inline numbered markers, three citation cards (number, time, meeting title, speaker,
      two-line quote, ribbon with pin). Done in #136 (rest), #636 (server data) and #624 (web). Markers are
      superscript links to the matching card (mapped by the citation's `n`); while streaming they are plain numbers
      because the cards arrive last, and a marker with no card is dropped, never shown raw. A document card has no
      ribbon. Checked by eye on 2026-11-26 at 1280 and 390 px, Daylight light and Ink dark, with the page served by
      Vite and the Ask and `/api/me` replies mocked (synthetic text, no server). Not compared side by side with the
      prototype Ask state; Daylight dark and Ink light were not shot.
- [x] Suggestion (#136) chips, sticky composer with Ask button, Cmd/Ctrl+K focus
- [x] States (#136): streaming, error, 401 "Sign in to ask.", rate limited (429), 503 "Ask needs a language model", no results

## S5 Upload
- [x] Drop zone with accepted types and size limit (only formats the backend accepts). Built in #138. The lede names no speech provider (jsdom test).
      Checked by eye on 2026-11-20 (#328) at 1280 and 390 px against the prototype Upload state, Daylight light. The
      listed formats (MP3, M4A, AAC, WAV, MP4, OGG, FLAC, MKV) are the ones `crates/speech` decodes and the 2 GB
      limit is `MAX_UPLOAD_BYTES` in `routes.rs`; the prototype lists only four and its lede adds "up to 4 hours" and
      "Processed on your server", which the app does not claim. The browser's own file button is unstyled in both.
- [x] Fields: title, language, speaker count; button "Process recording". The language select (Vietnamese default, English, Mixed languages) is built from #245 (checked by eye at 1280 px, Daylight light, no prototype side-by-side); speaker count is still hidden. Built in #138. Only title is built: the upload API reads
      no language or speaker count, so those are hidden rather than faked.
      Checked by eye on 2026-11-20 (#328) at 1280 and 390 px: the fields stack under the drop zone at 390 px and the
      button stays disabled until a file is chosen. The prototype calls the third language "Vietnamese + English";
      the app says "Mixed languages". The prototype's Number of speakers field is hidden on purpose.
- [ ] Pipeline progress over the real statuses (uploaded, transcribing, reporting, ready, failed); error with retry;
      link to the new meeting when ready. Built in #138. Retry queues the failed step again (`POST /api/meetings/{id}/retry`); no re-upload. Shows 4 steps (Uploaded, Transcribing, Report, Ready), not the prototype's 5, since the backend has no Speakers or Indexed status.
      Not ticked: on 2026-11-20 (#328) a real 1.4 MB WAV was uploaded to a local server whose speech key was
      invalid. The four steps and the red error ("Processing failed: speech provider refused the request: HTTP 401
      Unauthorized") showed, and pressing "Process recording" again re-uploaded the file (a second failed meeting,
      deleted afterwards). The error runs into the retry hint with no full stop between them. The link to a finished
      meeting was not exercised: it needs a working speech key.

## S6 Live meeting (built from the spec in #194, not checked against the prototype: it is not in the repo)
- [ ] Recording pill with timer, platform, join time, title; Pause; Stop and leave with confirm
      Open: title, "Recording" badge (and "Call ended"), platform, join time and an elapsed timer are built (#276; the
      timer stops at the final length when the call ends). jsdom test only: no screenshot, and the prototype has no
      live state in the repo. Pause and Stop and leave have no backend route, so they are hidden.
- [ ] Live ribbon, live transcript, speaking now, talk time, noted so far
      Open: the live transcript is built (lines with the time into the call, the partial line in grey, click a line to
      ask about it). The ribbon, speaking now, talk time and noted so far are not: the live stream has no speaker
      labels or notes (`docs/decisions/hosted-speech.md`).
- [ ] Live Ask panel ("Only you see these"): context line (last 60 s, or a clicked transcript line, with "use last 60 s"),
      optional extra words, Ask now button and `A` hotkey; answer cards newest first with asker, quote, time taken, answer,
      and Document or Meeting sources with page or time; "Passages only" card for documents not sent to the language model;
      waiting state (`docs/specs/live-ask.md`)
      Open: built with jsdom tests, no screenshot and no prototype side-by-side. Cards show the context, time taken,
      answer and sources (document or meeting with its time); there is no asker (only you see them) and no page number
      (documents have none). The answer arrives in one piece, not token by token. The page has not been run in a
      browser against a real call.

## S10 Browser recording (no prototype state exists: built from `web/src/ui/` primitives and the Upload screen's fields, `docs/specs/browser-recording.md`)
- [x] Meetings header: "Record from this browser" (primary) beside "Send notetaker". Checked in desktop Chrome at 1280 px, Daylight light, in #294; no prototype side-by-side (none exists)
- [x] Dialog: title, language (as on Upload), mode (Meeting tab / In-room, `Choice`), the consent box, the notice to copy for the call chat; Start recording stays disabled until the box is ticked and a title is set. A refused share shows its reason in the dialog. Checked in desktop Chrome at 1280 px, Daylight light, in #294
- [ ] Browsers without tab audio (no Chromium `userAgentData`, or mobile) see a message linking to Upload instead of the form. Open: jsdom test only, not opened in Safari or Firefox
- [x] Recorder bar in the live view, only in the tab that is recording: elapsed time, microphone and tab level meters (native `meter`), Stop, Stop and discard (shared `ConfirmDialog`), a warning when the shared tab's audio ends, a reconnecting note, lost-audio note, and a leave-page warning. Checked in desktop Chrome at 1280 px, Daylight light, in #294 (Stop, Stop and discard, meters, elapsed time); the tab-ended, reconnecting and lost-audio notes are jsdom only
      Not checked: mobile (390 px), the Ink skin and dark mode.

- [ ] Desktop app dialog (#347): modes Meeting app (headphones and "turn off other music and videos" hint) and In-room, tab mode hidden; a missing permission says where to grant it; the recorder bar meter reads "Audio". jsdom only: not opened in the app, no prototype screenshot, 390 px and Ink not checked
- [ ] Desktop meeting prompt (#383): Record on the prompt opens this dialog titled after the calendar event on now (or "<app> call HH:MM"), consent unticked, with a warning when the notetaker is already recording; the recorder bar of such a recording shows "The call in <app> ended. Recording stops in N s." with Keep recording. jsdom only: the prototype has no such screen, and it was not opened in the app
## S9 My documents (checked against the prototype My documents screen, version 8, 2026-11-19)
- [x] Drop zone: Markdown and TXT, up to 20 MB each, done, with the 22 px upload icon above the title like the Upload screen. PDF is not accepted: the server only stores Markdown and TXT (`crates/server/src/routes/documents.rs`), so a PDF row waits for PDF support there. Checked in a browser in #269
- [x] Table: name, type, size, added, status (Indexed, Indexing, No text found), the `role=switch` "May be sent to the language model" (off by default, "n/a" when no text) and Delete are done. Delete opens the shared modal confirm; the prototype confirms inline in the row (deliberate difference). Checked in a browser at 1280 px (Daylight and Ink, light and dark): the table now fits its card, so Delete is in view with no sideways scroll. At 390 px (same four skins) each row is a two-column card with labelled values (Status, switch and Delete in view, page width 390 px), instead of the prototype's sideways-scrolling table; this is a deliberate difference. Update 2026-11-26 (#631): the table now sits in its own full-width card below the upload and storage cards, so it shows as columns again at 1280 px (it had fallen back to cards in the narrow main column); the file picker button is styled like a Button on Documents and Upload. Checked in Chromium at 1280 and 390 px, Daylight and Ink, light and dark, against a mocked API. Screenshots in the PR (#269; the screenshots are in the project thread, not on the PR)
- [x] Aside: storage meter (used of 500 MB) above the explainer with the prototype's spacing; the bar has a minimum width so a few KB still shows. The explainer names no provider. Checked in a browser in #269
- [x] Rail: icon beside every item (Meetings, Upload, Ask, My documents, Live meeting, Recording & privacy), also in the mobile row; groups Work and Workspace. The prototype's Insights group is not shown: none of its screens exist in the app. Breadcrumb "Work / My documents" matches. Checked in a browser in #269
      Not checked in a browser: the empty, loading and error states, a nearly full meter and the Delete dialog.

## S7 Members
- [x] Invite form (email, role) with validation errors, and a Copy link button for the sign-in link the server returns; the notice says whether the invite was emailed, email is not set up, or the email failed (#633, jsdom tests; not checked in a browser). Members never see the form: the page shows "Only an admin can manage members." and never requests `/api/members`; the rail item and the account menu's "Manage members" show for admins only (jsdom tests). Checked in Chromium on 2026-11-26 at 1280 px (Daylight light) and 390 px (Daylight and Ink light) against a mocked API; the prototype was not opened side by side, so no prototype screenshot (written reason: compared with the Documents and Recording screens' shared `frame`, `Table` and `Field` only)
- [x] Members table: face, name, email, role select, meetings, pending badge, Remove or Cancel invite with inline confirm ("Yes, remove" / "Keep"; the signed-in admin has no Remove). At 390 px each row is a two-column card (shared `Table`). Same check as above; Ink dark and Daylight dark not checked. 2026-11-26 (#629): `inputClass` now has `Button`'s height (py-1.5), the Invite button sits on the inputs' row even when the email error shows, the pending row's role is a disabled select like the others, and the select text lines up with its column header. Checked in Chromium at 1280 and 390 px in Daylight and Ink, light and dark, against a mocked API. Recording & privacy was not rendered with the mock (blank page), so its inputs are checked by code only (same `inputClass`)
- [ ] Workspace switcher (#435): the account menu lists the person's workspaces under "Workspaces" when there are two or more, with a check on the current one; picking another returns to the meetings list with every cached answer dropped; the rail shows the workspace name under the wordmark (desktop only, hidden in the 390 px row). The prototype has one workspace and no switcher, so there is no prototype screenshot to compare with (written reason). Checked in Chromium on 2026-11-26 against a mocked API at 1280 px (Ink light) and 390 px (Daylight light, menu below the avatar); Ink dark and Daylight dark not checked
- [ ] Default sharing and Who can sign in radio groups: not built. `docs/specs/workspaces.md` has no setting behind either (the sign-in rule is the invite list), so there is nothing to save

## S8 Recording & privacy
- [x] Calendar row: Google Calendar tile with Connected, Reconnect needed or Not connected, Connect or Reconnect, Disconnect with confirm, and the `?calendar=` outcome message (both skins, mobile checked by eye)
- [ ] Notetaker sign-in status (admins only): a "Notetaker account" row from `GET /api/notetaker/status` with a Signed in, Signed out or Not checked yet badge, a red banner when signed out, and the `recapz notetaker-login` hint; members never request it (jsdom tests only; not checked in a browser or against the prototype, which has no such state)
- [x] Join rule radios, notetaker name, join message: admins edit and Save (`PUT /api/settings`, saved toast, server refusals next to the field) with a live preview of the chat notice that adds the "opt out" wording like the server; members read text only (jsdom tests). Name, message and retention fields fill the column (was: shrunk to default width). Checked in Chromium on 2026-11-26 at 1280 and 390 px, Daylight light. All four sections use the shared `.frame` rule: cards in Daylight, unframed rows with a hairline between them in Ink (prototype `.setbox` is framed only in Daylight); Ink light at 1280 px and Ink dark at 390 px checked by eye.
- [ ] Retention selects (audio, transcripts); "Where data lives" read-only text
      Open: a "Where audio goes" card names no speech provider (jsdom test). It says where recordings are kept (object storage or local disk) and how long their audio stays (read from `GET /api/settings/audio-retention`); checked by eye at 1280 px only. Admins get a "Delete audio after" select (30, 90, 180, 365 days, the current value, or keep until the meeting is deleted) that saves with `PUT` and toasts; a refusal is shown; members see text only (jsdom tests); checked in a headless browser at 1280 and 390 px, Daylight light, against mocked API data, 2026-11-19, with no prototype state to compare against. Admins also get a "Delete meetings after" select (90 days, 1 year, keep; `PUT /api/settings/transcript-retention`) that hard deletes whole meetings at the hourly sweep, and Report Details shows "Deleted automatically on <date>" (jsdom tests only, not checked in a browser or against a prototype state). The rest of "Where data lives" is unbuilt: no settings API for it yet. Earlier note: transcript retention and the rest of "Where data lives" have no settings API yet. Admins also get a "Usage this month" card (cost, cap, cap alert, cost per meeting) from `GET /api/usage`; members never request it; checked by eye at 1280 px only, no prototype state to compare.
- [ ] Delete everything one participant said, with confirm; Save changes; saved toast
      Open: admins get a "Delete everything one participant said" card (person select, confirm naming the person and meeting count, toast; jsdom tests). Not checked in a browser or against the prototype (written reason: the prototype has no such state and no browser run was possible here); Save changes is not built.
- [ ] Spoken language select on a scheduled meeting's page (not in the prototype): jsdom test only, no screenshots at 1280 or 390 px, both skins, so nothing to compare against.
