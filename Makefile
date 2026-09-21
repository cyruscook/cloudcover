.PHONY: fmt build check test site-data site-build

fmt:
	cargo fmt --all

build:
	cargo build --workspace

check:
	cargo fmt --all -- --check
	cargo check --workspace
	cargo clippy --workspace --all-targets -- -D warnings
	npm --prefix site run check

test:
	cargo test --workspace

site-data:
	npm --prefix site run generate:data

site-build:
	npm --prefix site run build
