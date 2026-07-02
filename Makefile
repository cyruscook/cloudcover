.PHONY: fmt fmt-check build check lint test

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

build:
	cargo build --workspace

check:
	cargo check --workspace

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace
