.PHONY: dev test lint release

dev:
	@trap 'kill 0' INT TERM; cargo run -p recapz-server & pnpm -C web dev & wait

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
