.PHONY: dev local hooks test test-browser lint release

dev:
	@trap 'kill 0' INT TERM; cargo run -p recapz-server & pnpm -C web dev & wait

local:
	@test -f config.toml || { echo "copy config.example.toml to config.toml first (see docs/running-locally.md)"; exit 1; }
	@trap 'kill 0' INT TERM; cargo run -p recapz-server --features whisper & pnpm -C web dev & wait

# Once per clone: the pre-push hook refuses pushes to main.
hooks:
	git config core.hooksPath scripts/git-hooks

test:
	./scripts/test-push-guards.sh
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
	cargo build --release
