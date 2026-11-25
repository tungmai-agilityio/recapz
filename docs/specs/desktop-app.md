# Desktop app

Status: **agreed with the maintainer (2026-11-23)**
Intent: #340

## Problem

Browser recording (`docs/specs/browser-recording.md`) only hears a Chrome or Edge tab. Calls in the Zoom, Teams or
Slack desktop apps, and Meet in Safari, can only be recorded with the microphone ("In-room"), so the far side is
heard faintly or not at all. The notetaker covers Meet only and depends on one saved Google sign-in.

What already exists:

- The shared Recapz Build Guide (section 12b) and Build Plan put a Tauri desktop recorder in Phase 2, after v1. They
  planned a resumable chunked upload, device tokens in the keychain, Opus chunks on disk and local diarization with
  sherpa-onnx. The Build Plan gives the timing two ways (weeks 9–14 and 13–16).
- The prototype has no desktop screens.
- Since those documents were written, browser recording has shipped: `POST /api/recordings` plus the
  `GET /api/meetings/{id}/record` WebSocket stream 16 kHz mono frames, the server writes the WAV as they arrive,
  and the live view, Live Ask and speaker labels from the speech provider all work. Most of the old server plan is
  no longer needed.

## Decisions

- **Team only:** one build per server. `base_url` is fixed at build time. The app updates itself (see
  "Auto-update"); the first build with the updater is installed by hand once.
- **Tauri 2, macOS 14.2+ only:** a thin shell in `desktop/`, in its own Cargo workspace so the server's Linux CI
  never builds WebKit. There is no ScreenCaptureKit fallback, so a member on macOS 13 upgrades. Windows is a later
  spec.
- **The window is the Recapz web app:** the webview loads `base_url`. Because it is the same origin, the session
  cookie, CSRF, the socket's origin check and every page work unchanged, and there is no second UI to keep in line
  with the prototype.
- **All capture is native:** Rust captures system audio with a global Core Audio process tap (no Screen Recording
  permission) and the microphone with Core Audio. It mixes the two to 16 kHz mono i16 and passes the frames to the
  page over a Tauri channel. The page only forwards them to the existing socket, so the 60-second reconnect
  buffer, consent box and Stop/discard stay one code path. The browser worklet is untouched. Native mic capture
  avoids WKWebView asking for microphone permission again for a remote origin.
- **Device changes survive:** the tap is global, not bound to one output device, and the mic reopens when the
  default input changes. Switching to AirPods mid-call leaves no gap over 2 s.
- **Echo:** browser `echoCancellation` cannot cancel Zoom's output. With speakers, the far side would be heard
  twice, so the dialog recommends headphones, as in browser recording. Ticket 3b runs a timeboxed spike on whether
  macOS VoiceProcessingIO on the mic cancels system output, and it is adopted only if it does.
- **Streaming while hidden:** the page keeps owning the socket. While recording, Rust holds an App Nap opt-out
  (`NSProcessInfo.beginActivity`), and closing the window minimizes it instead of hiding it. If a minimized,
  covered window drops audio, the socket moves to Rust in a follow-up.
- **Record dialog modes in the app:** "Meeting app" (system audio + mic) and "In-room" (mic only, native). The
  tab mode is hidden because the webview has no tab picker and system audio covers browser calls too.
- **Everything the Mac plays is recorded:** the global tap also captures music, videos and notification sounds,
  not only the call. The "Meeting app" option tells the member to turn off other music and videos while
  recording. Picking one app to record is not in v1, because a global tap keeps recording through device changes.
- **No server recording changes:** the desktop sends the same frames as a browser recording, so `platform =
  'browser'` and the usage-cap rules carry over. The server only validates `mode`: "Meeting app" sends `"tab"`
  and "In-room" sends `"room"`.
- **Sign-in through the system browser:** Google blocks OAuth inside embedded webviews (`disallowed_useragent`).
  The app opens Google sign-in in the default browser. The server then redirects to `recapz://signed-in?code=…`
  with a one-time code (single use, 60 s TTL, stored hashed), and the webview exchanges that code for the normal
  session cookie. This is the only new server code.
- **IPC limited to one origin:** the Tauri capability allows remote IPC only from `base_url`, and only for the
  audio commands (start, stop, levels, permission).
- **Presence:** a Dock app with a menu-bar icon (Open, Quit, and a red dot while recording).
- **Consent unchanged:** the same "I have told everyone…" box and copyable notice.
- **Local builds, self-signed:** there is no Apple Developer account yet and no macOS CI (the free plan bills
  macOS minutes at 10×). The maintainer signs with a self-signed "Recapz Local" code-signing certificate from
  Keychain, so the mic and audio-capture grants survive updates. Members right-click → Open once. `make
  desktop-dmg` builds it, and the `.dmg` is attached to the GitHub release.
- **Checks without CI:** the pre-push hook runs `cargo fmt --check` and Clippy in `desktop/` when `desktop/**`
  changed, and `test-runner` runs the same before a PR.

## Out of scope

- Meeting detection (mic in use by Zoom or Teams → "Record this meeting?"): `docs/specs/meeting-detection.md`.
- Speaker names from the Zoom or Teams UI via Accessibility, and typing the notice into the call chat.
- Separate "me" and "others" channels, and local diarization (the speech provider already labels speakers).
- Spooling to disk for outages longer than 60 s, and offline recording.
- Developer ID signing, notarization and macOS CI (until an Apple account is bought).
- Windows, Linux, mobile, macOS 13, a Mac App Store build, and other teams' servers.

## Acceptance

- [ ] A member signs in once through the system browser and lands signed in inside the app. The code works once and
      expires after 60 s.
- [ ] In the app, Record offers "Meeting app" and "In-room", and not the tab mode. "Meeting app" shows the
      headphones and "turn off other sounds" hints. A Zoom desktop call and a Meet call in Safari, each recorded
      with headphones, have both sides audible, and the live view and meeting report work as for a browser
      recording.
- [ ] Without the mic or audio-capture permission, the dialog says how to grant it, offers "Open System Settings"
      (opens the Microphone or System Audio Recording pane), and does not start.
- [ ] A recording with the window minimized and fully covered for 30 minutes has no gaps.
- [ ] Switching output to AirPods mid-call leaves no gap over 2 s.
- [ ] The menu bar shows a recording indicator for the whole recording.
- [ ] Remote IPC is refused for any origin except `base_url`.
- [ ] A `.dmg` from `make desktop-dmg` installs on a clean macOS 14.2 machine after right-click → Open, and the
      permissions persist across an update to the next build.
- [ ] `docs/spec.md` names the desktop app, and `docs/ui-parity.md` has rows for the desktop dialog modes.

## Tickets

| # | Title (`type(scope): summary`) | Area | Blocked by | Touches | Size | Testing scope |
|---|---|---|---|---|---|---|
| 1 (#343) | feat(server): sign in the desktop app with a one-time code | server, auth | none | `crates/server/src/routes.rs`, `crates/server/src/routes/desktop_auth.rs`, `crates/server/tests/desktop_auth.rs` | small | `cargo test --workspace` (auth) |
| 2 (#344) | feat(desktop): open Recapz in a Tauri shell | desktop | 1 | `desktop/`, `Makefile`, `scripts/git-hooks/pre-push` | medium | `cargo clippy` + build in `desktop/`, manual launch |
| 3a (#345) | feat(desktop): capture macOS system audio | desktop | 2 | `desktop/src-tauri/src/audio/tap.rs`, `desktop/src-tauri/Info.plist`, `desktop/src-tauri/capabilities/` | medium | unit test on the resampler/framer, manual Zoom call |
| 3b (#346) | feat(desktop): capture and mix the microphone | desktop | 3a | `desktop/src-tauri/src/audio/mic.rs`, `desktop/src-tauri/src/audio/mix.rs`, `desktop/src-tauri/Info.plist` | medium | unit test on the mixer, manual device swap |
| 4 (#347) | feat(web): record in the desktop app | web | 3b | `web/src/capture/`, `web/src/pages/Record*.tsx`, `docs/spec.md`, `docs/ui-parity.md` | small | `pnpm -C web test`, lint, manual flow in the app |
| 5 (#348) | chore(deploy): build the macOS app locally and attach it to the release | deploy | 2 | `Makefile`, `.claude/skills/release/`, `docs/deploy.md` | small | `make desktop-dmg` on the maintainer's Mac |

1. `GET /auth/desktop` runs the normal Google sign-in in the system browser, then redirects to
   `recapz://signed-in?code=…&state=…` (the app's own random `state`, passed in as `/auth/desktop?state=`).
   `POST /auth/desktop/exchange` (same-origin, CSRF-checked, code and state must match) swaps the code for the
   usual session cookie. Codes are hashed, single use and expire after 60 s. Critical by `CLAUDE.md`:
   `human-merge`.
2. Tauri 2 app: one window on `base_url`, the deep-link and single-instance plugins (reason: the sign-in
   redirect), a Dock icon, and a menu-bar icon with Open and Quit. Close closes the window (the app stays in the Dock and menu bar; while recording it asks first). Quit always exits and stops capture. `make desktop-check` runs fmt
   and Clippy, and the pre-push hook calls it when `desktop/**` changed.
3a. Global Core Audio process tap → 16 kHz mono i16 frames of 1600 samples → a `tauri::ipc::Channel`. Commands
   `audio_start`, `audio_stop`, `audio_permission`, `open_privacy_settings` (opens only the microphone or audio-capture
   Privacy pane). Holds the App Nap opt-out while recording. The capability
   scope is `remote.urls = [base_url]`.
3b. Mic capture on the default input, reopened when the default changes, plus a sum-and-clamp mixer with the
   tap. `audio_start` takes `mode: meeting | in_room`. Timeboxed VoiceProcessingIO spike, with the result
   written in the PR. It is adopted only if it cancels Zoom's output.
4. When `window.__TAURI__` exists, the Record dialog shows "Meeting app" and "In-room" and hides the tab mode.
   "Meeting app" carries the headphones and "turn off other music and videos" hints. The native frames go to the socket in place of the worklet output, and the buffer and recorder bar are unchanged.
   The recorder bar's "tab audio ended" warning becomes "system audio stopped". The menu-bar red dot follows the
   recording state.
5. A one-time script or doc step creates the "Recapz Local" self-signed certificate. `make desktop-dmg` builds and
   signs with it. The release skill's checklist gains `gh release upload <tag> Recapz.dmg`.

## Auto-update

Decided 2026-11-25. This replaces the earlier "no auto-update" decision. Web changes never need it: the window opens
the bundled `ui/index.html`, which probes `/api/health` and then switches to the server, so a server deploy shows
the new UI on the next page load. Only changes to the Rust shell, the bundled `ui/index.html`, the IPC permissions or
`base_url` need a new app build.

- **Mechanism:** the Tauri 2 updater plugin (`tauri-plugin-updater`) with `bundle.createUpdaterArtifacts: true`. The
  check, download and install run in Rust only (`check`, `download_and_install`, then restart). No `updater:*`
  permission is added to any capability, so the remote page cannot reach the updater and the `base-url` capability
  stays as it is.
- **Manifest:** a static `latest.json` (`version`, `notes`, `pub_date`, `platforms.darwin-aarch64`, and
  `darwin-x86_64` only if the build is universal; each with `signature` = the `.sig` file content and `url` = the
  release asset). It is hosted as a GitHub release asset and read from
  `https://github.com/tungmai-agilityio/recapz/releases/latest/download/latest.json`. The repository is public, so
  no token is needed or ever embedded in the app. If the repository becomes private, move the endpoint to the
  Recapz server rather than shipping a GitHub token.
- **Endpoint override:** the endpoint comes from a build variable (like `RECAPZ_BASE_URL`) with the URL above as the
  default, so a test build can point at a local server.
- **Versions:** the updater compares semver, so each build gets its version from the release tag instead of the fixed
  `0.1.0` in `tauri.conf.json`. A build without a higher version is never offered.
- **Release artifacts:** `Recapz.dmg` (first install), `Recapz.app.tar.gz`, `Recapz.app.tar.gz.sig` and `latest.json`.
  The newest release must always carry a `latest.json`, because `releases/latest` follows the newest release. When
  `desktop/` did not change, the release step copies the previous `latest.json` (its absolute URLs still point at
  the older assets); otherwise the updater gets a 404 until the next desktop release.
- **Signing key:** `cargo tauri signer generate -w ~/.tauri/recapz-updater.key`. The private key and its password
  live only on the maintainer's Mac and in a password manager. They never enter the repository, chat, logs or CI
  (there is no macOS CI). `plugins.updater.pubkey` holds the public key and is committed. The build reads
  `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from the environment and stops with a clear
  message when the key is missing. Losing the key means installed apps can no longer be updated and every member
  installs again by hand with a new key. The update signature is independent of the "Recapz Local" code signature,
  so no Developer ID is needed.
- **When it checks:** at launch, then every 6 hours while the app runs, and from a "Check for Updates…" item in the
  menu bar. A network error is logged and shows nothing.
- **What the member sees:** with no newer version, nothing (the menu item says "up to date"). With one, the app
  downloads it in the background and asks "Recapz x.y.z is ready. Restart now / Later". "Later" installs it on the
  next launch.
- **Never during a recording:** while `RECORDING` is on, the app neither asks nor restarts. The prompt waits until
  the recording stops. A restart must never cut a recording.
- **No rollback:** a broken update is not rolled back. The maintainer updates their own Mac first and uploads
  `latest.json` last; a build that cannot open is fixed by a new release and, if needed, a manual install.

### Unverified assumptions

Each of these is checked in the end-to-end test (PR 4); none is verified yet.

- An update downloaded by the app's own HTTP client is not quarantined (`com.apple.quarantine` absent per
  `xattr -l`), so Gatekeeper does not ask again. The first install still needs right-click → Open (macOS 15 may
  need Privacy & Security → Open Anyway); the updater does not change that.
- Microphone and system-audio grants survive an update because every build is signed with the same "Recapz Local"
  certificate (also the open acceptance row above).
- The updater can replace `/Applications/Recapz.app` for a member who is not an admin. The Tauri docs are silent;
  if it fails the app must show an error and the dmg install remains the fallback.
- The `darwin-aarch64` key is enough because the maintainer's Mac is Apple silicon; add `darwin-x86_64` only for a
  universal build.

### Acceptance (auto-update)

- [ ] **PR 2, shell:** `make desktop-check` is clean. With no newer version, nothing is shown. A newer version is
      downloaded and offered with Restart now / Later. While recording, no prompt appears until the recording
      stops. The remote capability has no `updater:*` permission (test or config check). A missing or invalid
      signature is rejected.
- [ ] **PR 3, release:** `make desktop-dmg VERSION=x.y.z` writes that version into the build and produces
      `.app.tar.gz`, `.sig` and `latest.json`, and stops with a clear message without
      `TAURI_SIGNING_PRIVATE_KEY`. The release step uploads all four files, or copies the previous `latest.json` when
      `desktop/` is unchanged. The `latest.json` of the newest release downloads, and its `signature` equals the `.sig`
      file.
- [ ] **PR 4, end to end:** `make desktop-update-test` builds 0.0.1 (endpoint `http://localhost:8765/latest.json`,
      insecure transport allowed in that test build only) and 0.0.2, and serves them locally. Opening 0.0.1 prompts,
      Restart now lands on 0.0.2, the microphone and system-audio grants remain, and `xattr -l` shows no quarantine.
      The unverified assumptions above are each marked confirmed or refuted.

## Open questions

- **Apple Developer account ($99/year):** deferred. Buy it when members find right-click → Open painful, or when
  the team grows. It brings Developer ID, notarization and a tag build on a macOS runner.
- **VoiceProcessingIO:** does it cancel system output on the mic? Answered by 3b's spike.
- **Hidden-window streaming:** if the 30-minute test fails, move the socket to Rust (cookie and 60 s buffer in
  Rust).
- **Windows:** check whether WebView2 `getDisplayMedia` gives system audio before writing a Windows spec.
- **Disk spool:** do members record on flaky Wi-Fi often enough to need audio spooled to disk beyond 60 s? Decide
  after a month of use.
