# Meeting detection

Status: **agreed with the maintainer (2026-11-25); tickets 1 to 3 merged (#378, #379, #392).** Only unit tests and jsdom
tests ran. No real Zoom, Teams or Meet call has been checked, so every acceptance row below is *unverified*. The
optional ticket to read the Meet link from the browser (#384) is backlog and not part of v1.

## Problem

The desktop app records only when the member opens the Record dialog. A member who joins a Zoom, Teams or Slack
call, or a Meet call in a browser, has to remember to start Recapz, and often does not.

## Decisions

- **Detect by who holds the microphone:** Core Audio lists every process doing audio and whether it reads input
  (`kAudioHardwarePropertyProcessObjectList`, `kAudioProcessPropertyIsRunningInput`, macOS 14+). No new permission,
  and the app never opens the microphone to detect. Recapz's own capture is excluded by pid.
- **Known apps only:** Zoom, Microsoft Teams, Slack, Webex, FaceTime, Discord, Chrome, Edge, Arc and Safari, by
  bundle id; a helper (`com.google.Chrome.helper`) counts as its app. Any other app holding the microphone is ignored.
- **Browsers by microphone only:** a browser holding the microphone is treated as a call. No Apple Events or
  Automation to read the tab URL, no browser extension, no Accessibility or Screen Recording in v1.
- **Start and end:** a call starts once an app held the microphone 5 s without a break, and ends once it released
  it for 15 s (muting or a device switch is not the end). Polled every 2 s.
- **Always ask:** "Record this meeting?" with Record / Ignore / Never ask again for this app. Record opens the
  Record dialog prefilled ("Meeting app" mode, a title); the consent tick stays required. No auto-record in v1.
- **Never interrupt:** no prompt while a recording runs.
- **On by default:** a "Detect Meetings" switch in the menu bar, stored on the Mac. "Ask Again for All Apps" in
  the same menu clears the never list.
- **Auto-stop:** only a recording started from the prompt; when its app releases the microphone, the recorder bar
  counts down 10 s with "Keep recording".
- **Notetaker overlap:** when a calendar meeting is on now and the notetaker is joining or recording it, the
  dialog warns before recording it twice. Without the tab URL this is matched by time, so it warns and does not block.

## Out of scope

- Reading the Meet link from the browser, a browser extension, speaker names or chat notices via Accessibility.
- Auto-record.
- Windows.

## Acceptance

- [ ] A Zoom call and a Meet call in Chrome are each noticed within 7 s, with no new permission prompt.
- [ ] The prompt never appears while recording, and appears once per call.
- [ ] Record opens the dialog prefilled with the consent box unticked; Ignore and Never ask again behave as named,
      and the never list survives a restart.
- [ ] A recording started from the prompt stops 10 s after the call ends unless "Keep recording" is pressed; a
      manual recording is never stopped.
- [ ] With the notetaker in a calendar meeting that is on now, Record warns first.

## Tickets

| # | Title (`type(scope): summary`) | Area | Blocked by | Touches | Size | Testing scope |
|---|---|---|---|---|---|---|
| 1 (#378) | feat(desktop): notice which app holds the microphone | desktop | none | `desktop/src-tauri/src/detect.rs`, `main.rs`, this spec | small | unit tests on the detector, manual call |
| 2 (#379) | feat(desktop): ask "Record this meeting?" | desktop | 1 | `detect.rs`, a bundled prompt window, menu bar, local settings file | medium | unit tests, manual call |
| 3 (#392) | feat(web): open the Record dialog from a detected meeting | web | 2 | `web/src/pages/Record.tsx`, `web/src/capture/native.ts`, `docs/ui-parity.md` | small | `pnpm -C web test`, lint |

1. Lists the meeting apps holding the microphone and turns the samples into start and end events. Logs the app
   name only; nothing is shown yet.
2. A small always-on-top window from the bundled `ui/` (no remote IPC) with Record / Ignore / Never ask again,
   hidden after 30 s. The never list and the "Detect meetings" switch live in a JSON file in the app's config
   folder. Record shows the main window and dispatches `recapz-meeting-detected` to the page only when it is on
   `base_url`; the call's end dispatches `recapz-meeting-ended`.
3. The page opens the prefilled dialog on `recapz-meeting-detected`, warns on a notetaker overlap, and handles
   the auto-stop countdown.

## Open questions

- Do Zoom, Teams and Meet keep the microphone while muted? If not, a muted call would end early; checked on the
  first real call after ticket 1.
- Safari pages use the microphone through the shared WebKit GPU process, so another WebKit app could read as Safari.
