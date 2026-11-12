.PHONY: dev local test lint release

dev:
	@trap 'kill 0' INT TERM; cargo run -p recapz-server & pnpm -C web dev & wait

local:
	@test -f config.toml || { echo "copy config.example.toml to config.toml first (see docs/running-locally.md)"; exit 1; }
	@trap 'kill 0' INT TERM; cargo run -p recapz-server --features whisper & pnpm -C web dev & wait

test:
	cargo test --workspace
	pnpm -C web test

lint:
	cargo fmt --check
	cargo clippy --workspace --all-targets -- -D warnings
	pnpm -C web lint
	./scripts/banned-names.sh

release:
	pnpm -C web build
	cargo build --release
