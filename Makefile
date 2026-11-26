.PHONY: dev local hooks desktop-check desktop-dmg desktop-run desktop-update-test notetaker-login notetaker-logout sample live-sim replay live-stop pr-check test test-browser lint release

dev:
	@trap 'kill 0' INT TERM; set -a; [ -f .env ] && . ./.env; set +a; cargo run -p recapz-server & pnpm -C web dev & wait

local:
	@test -f config.toml || { echo "copy config.example.toml to config.toml first (see docs/running-locally.md)"; exit 1; }
	@trap 'kill 0' INT TERM; set -a; [ -f .env ] && . ./.env; set +a; cargo run -p recapz-server & pnpm -C web dev & wait

# Sign the notetaker's Google account in (opens a browser window), or delete that sign-in.
notetaker-login notetaker-logout:
	cargo run -p recapz-server -- $@

# Helpers for a running `make local` with dev_login = true. They sign in as bootstrap_admin_email from
# config.toml (override with EMAIL=...) and keep the cookies in data/dev-cookies.txt.
API ?= http://localhost:8080
EMAIL ?= $(shell awk -F'"' '/^bootstrap_admin_email/{print $$2}' config.toml 2>/dev/null)
SPEECH_LANG ?= vi
JAR := data/dev-cookies.txt
dev_login = mkdir -p data && curl -sS --fail-with-body -o /dev/null -c $(JAR) -H 'content-type: application/json' \
	-d '{"email":"$(EMAIL)"}' $(API)/auth/dev-login && CSRF=$$(awk '$$6=="recapz_csrf"{print $$7}' $(JAR))
dev_curl = curl -sS --fail-with-body -b $(JAR) -H "x-csrf-token: $$CSRF"

# Upload a short spoken sample (needs macOS `say` and ffmpeg).
sample:
	@mkdir -p data && say -o data/sample.aiff "Hi Sam, we launch on March fourteenth. Sam owns the payment testing."
	@ffmpeg -loglevel error -y -i data/sample.aiff -ar 16000 -ac 1 data/sample.wav
	@$(dev_login) && $(dev_curl) -F title=Sample -F file=@data/sample.wav $(API)/api/uploads && echo

# Play the built-in script into the live view as a fake call (no audio, no speech provider).
live-sim:
	@$(dev_login) && $(dev_curl) -H 'content-type: application/json' -d '{}' $(API)/api/dev/live-sim && echo

# Replay a recording as a fake call through the speech provider: make replay FILE=call.wav [SPEECH_LANG=vi]
replay:
	@test -n "$(FILE)" || { echo "usage: make replay FILE=call.wav [SPEECH_LANG=vi]"; exit 1; }
	@$(dev_login) && $(dev_curl) -F file=@$(FILE) -F language=$(SPEECH_LANG) -F title=Replay $(API)/api/dev/live-sim/audio && echo

# End a fake call: make live-stop ID=<meeting id> [OPTOUT=1 deletes it instead]
live-stop:
	@test -n "$(ID)" || { echo "usage: make live-stop ID=<meeting id> [OPTOUT=1]"; exit 1; }
	@$(dev_login) && $(dev_curl) -X DELETE "$(API)/api/dev/live-sim/$(ID)$(if $(OPTOUT),?optout=true)" && echo

# Branch, title and commit checks plus the size guide, before pushing. TITLE defaults to the last commit subject.
pr-check:
	./scripts/check-commits.sh origin/main..HEAD "$(or $(TITLE),$$(git log -1 --format=%s))" "$$(git branch --show-current)"
	node scripts/pr-size.mjs origin/main

# The macOS app (its own workspace, never built by CI). Build it with RECAPZ_BASE_URL=https://your.server.
desktop-check:
	cd desktop && cargo fmt --check && cargo clippy --all-targets -- -D warnings

# Signed Recapz.dmg plus the update files for one release (macOS only; run scripts/desktop-cert.sh once first).
# `make desktop-dmg VERSION=0.1.3` (the release tag without the v). The URL is baked in. The updater key stays
# on this Mac: export TAURI_SIGNING_PRIVATE_KEY_PASSWORD, and set UPDATER_KEY if it is not at the default path.
# Needs `cargo install tauri-cli --version '^2' --locked`.
DESKTOP_BASE_URL ?= https://recapz.tungmai.com
UPDATER_KEY ?= $(HOME)/.tauri/recapz-updater.key
BUNDLE = desktop/target/release/bundle
desktop-dmg:
	@test -n "$(VERSION)" || { echo "usage: make desktop-dmg VERSION=x.y.z (the release tag without the v)"; exit 1; }
	@test -f "$(UPDATER_KEY)" || { echo "no updater key at $(UPDATER_KEY): run cargo tauri signer generate -w $(UPDATER_KEY) once"; exit 1; }
	@cargo tauri --version >/dev/null 2>&1 || { echo "cargo tauri is missing: run cargo install tauri-cli --version \"^2\" --locked once"; exit 1; }
	@security find-certificate -c "Recapz Local" >/dev/null 2>&1 || { echo "no 'Recapz Local' certificate: run scripts/desktop-cert.sh once"; exit 1; }
	# CI=true makes the Tauri bundler pass --skip-jenkins, so create-dmg skips the Finder AppleScript (needs an Automation permission).
	cd desktop/src-tauri && CI=true RECAPZ_BASE_URL=$(DESKTOP_BASE_URL) APPLE_SIGNING_IDENTITY="Recapz Local" TAURI_SIGNING_PRIVATE_KEY="$$(cat "$(UPDATER_KEY)")" cargo tauri build --bundles app,dmg --config '{"version":"$(VERSION)"}'
	cp "$$(ls -t $(BUNDLE)/dmg/*.dmg | head -1)" Recapz.dmg
	cp $(BUNDLE)/macos/Recapz.app.tar.gz $(BUNDLE)/macos/Recapz.app.tar.gz.sig .
	./scripts/desktop-latest.sh $(VERSION) Recapz.app.tar.gz.sig > latest.json
	codesign --verify --deep --strict --verbose=2 "$$(ls -d $(BUNDLE)/macos/*.app | head -1)"
	@echo "ready: gh release upload v$(VERSION) Recapz.dmg Recapz.app.tar.gz Recapz.app.tar.gz.sig latest.json (latest.json last)"

# Build the app, install it over /Applications/Recapz.app and open it (macOS only). Same "Recapz Local" certificate as
# desktop-dmg, so the microphone and audio grants stay. No updater key needed: the update files are skipped and the update URL
# is a dead local port, so a published release never replaces this build. The URL is baked in at build time.
#   make desktop-run                    the local web app, http://localhost:5173 (`make local`)
#   make desktop-run TARGET=prod        the hosted server, https://recapz.tungmai.com
#   make desktop-run URL=https://...    any other server (DESKTOP_BASE_URL=... works too)
#   make desktop-run SKIP_BUILD=1       reuse the last build and only reinstall and relaunch
#   make desktop-run HELP=1             print this usage
DESKTOP_RUN_URL = $(or $(URL),$(if $(filter environment command line,$(origin DESKTOP_BASE_URL)),$(DESKTOP_BASE_URL)),$(if $(filter prod,$(TARGET)),https://recapz.tungmai.com),http://localhost:5173)
DESKTOP_RUN_BUILD = cd desktop/src-tauri && CI=true RECAPZ_BASE_URL=$(DESKTOP_RUN_URL) RECAPZ_UPDATE_URL=http://localhost:1/latest.json APPLE_SIGNING_IDENTITY="Recapz Local" cargo tauri build --bundles app --config '{"version":"0.0.0","bundle":{"createUpdaterArtifacts":false}}'
desktop-run:
	@test -z "$(HELP)" || { echo "usage: make desktop-run [TARGET=prod | URL=https://...] [SKIP_BUILD=1]  (default: http://localhost:5173)"; exit 1; }
	@cargo tauri --version >/dev/null 2>&1 || { echo "cargo tauri is missing: run cargo install tauri-cli --version \"^2\" --locked once"; exit 1; }
	@security find-certificate -c "Recapz Local" >/dev/null 2>&1 || { echo "no 'Recapz Local' certificate: run scripts/desktop-cert.sh once"; exit 1; }
	$(if $(SKIP_BUILD),@echo "reusing the last build",$(DESKTOP_RUN_BUILD))
	-osascript -e 'tell application "Recapz" to quit'
	@sleep 2
	rm -rf /Applications/Recapz.app
	cp -R $(BUNDLE)/macos/Recapz.app /Applications/Recapz.app
	codesign --verify --deep --strict /Applications/Recapz.app
	open /Applications/Recapz.app
	@echo "Recapz now talks to $(if $(SKIP_BUILD),the URL of the last build,$(DESKTOP_RUN_URL))"

# Local end-to-end test of the updater: builds 0.0.1 and 0.0.2 and serves 0.0.2 on localhost:8765.
desktop-update-test:
	./scripts/desktop-update-test.sh

# Once per clone: the pre-push hook refuses pushes to main.
hooks:
	git config core.hooksPath scripts/git-hooks

test:
	./scripts/test-push-guards.sh
	./scripts/test-desktop-latest.sh
	./scripts/test-automerge.sh
	./scripts/test-health.sh
	cargo test --workspace
	pnpm -C web test

# Tests that start a real Chromium; prints SKIPPED when none is installed.
test-browser:
	cargo test -p recapz-notetaker --features browser-tests browser_tests -- --nocapture

lint:
	cargo fmt --check
	cargo clippy --workspace --all-targets -- -D warnings
	pnpm -C web lint

release:
	pnpm -C web build
	cargo build --release -p recapz-server
