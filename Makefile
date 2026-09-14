.PHONY: fmt build check test

fmt:
	cargo fmt --all

build:
	cargo build --workspace

check:
	cargo fmt --all -- --check
	cargo check --workspace
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace
