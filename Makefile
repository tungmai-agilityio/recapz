.PHONY: dev local hooks desktop-check desktop-dmgnotetaker-login notetaker-logout sample live-sim replay live-stop pr-check test test-browser lint release

dev:
	@trap 'kill 0' INT TERM; cargo run -p recapz-server & pnpm -C web dev & wait

local:
	@test -f config.toml || { echo "copy config.example.toml to config.toml first (see docs/running-locally.md)"; exit 1; }
	@trap 'kill 0' INT TERM; cargo run -p recapz-server & pnpm -C web dev & wait

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

# Signed Recapz.dmg for the release (macOS only; run scripts/desktop-cert.sh once first). The URL is baked in.
# Needs `cargo install tauri-cli --version '^2' --locked`.
DESKTOP_BASE_URL ?= https://recapz.tungmai.com
desktop-dmg:
	@cargo tauri --version >/dev/null 2>&1 || { echo "cargo tauri is missing: run cargo install tauri-cli --version \"^2\" --locked once"; exit 1; }
	@security find-certificate -c "Recapz Local" >/dev/null 2>&1 || { echo "no 'Recapz Local' certificate: run scripts/desktop-cert.sh once"; exit 1; }
	cd desktop/src-tauri && RECAPZ_BASE_URL=$(DESKTOP_BASE_URL) APPLE_SIGNING_IDENTITY="Recapz Local" cargo tauri build --bundles app,dmg
	cp "$$(ls -t desktop/target/release/bundle/dmg/*.dmg | head -1)" Recapz.dmg
	codesign --verify --deep --strict --verbose=2 "$$(ls -d desktop/target/release/bundle/macos/*.app | head -1)"
	@echo "Recapz.dmg ready: gh release upload <tag> Recapz.dmg"

# Once per clone: the pre-push hook refuses pushes to main.
hooks:
	git config core.hooksPath scripts/git-hooks

test:
	./scripts/test-push-guards.sh
	./scripts/test-automerge.sh
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
