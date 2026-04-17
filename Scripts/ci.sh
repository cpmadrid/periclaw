#!/usr/bin/env bash

# CI — the full set of checks CI runs, runnable locally.
# Invoked via `./dev ci`. Fails on any error.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "${SCRIPT_DIR}/lib.sh"
cd_to_project_root

require_bin cargo "Install Rust from https://rustup.rs"

print_step "cargo fmt --check"
cargo fmt --all -- --check
print_success "format clean"

print_step "cargo clippy --all-targets --all-features -- -D warnings"
cargo clippy --all-targets --all-features -- -D warnings
print_success "clippy clean"

print_step "cargo test"
cargo test
print_success "tests pass"

print_step "cargo build --release"
cargo build --release
print_success "release build succeeds"

print_success "CI pipeline passed"
