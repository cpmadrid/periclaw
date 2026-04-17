# Mission Control Desktop — dev tasks
# Run with: just <task>

default:
    @just --list

# Run the app in dev mode
dev:
    RUST_LOG=sassy_mc=debug cargo run

# Run with mock OpenClaw data (no WS connection to ubu-3xdv)
mock:
    OPENCLAW_MOCK=1 RUST_LOG=sassy_mc=debug cargo run

# Type-check without building
check:
    cargo check --all-targets

# Format
fmt:
    cargo fmt

# Lint
clippy:
    cargo clippy --all-targets -- -D warnings

# Release build, stripped
build-release:
    cargo build --release
    @echo "binary: target/release/sassy-mc ($(du -h target/release/sassy-mc | cut -f1))"

# Run unit tests
test:
    cargo test

# Cross-compile for ubu-3xdv (ARM64 Linux) — requires `cross` installed
build-arm64:
    cross build --release --target aarch64-unknown-linux-gnu
