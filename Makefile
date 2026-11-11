.PHONY: dev test lint release

dev:
	cargo run -p recapz-server

test:
	cargo test --workspace

lint:
	cargo fmt --check
	cargo clippy --workspace --all-targets -- -D warnings
	./scripts/banned-names.sh

release:
	cargo build --release
