#!/usr/bin/env bash

# Test — cargo test wrapper.
# Invoked via `./dev test`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "${SCRIPT_DIR}/lib.sh"
cd_to_project_root

require_bin cargo "Install Rust from https://rustup.rs"

EXTRA_ARGS=("$@")

print_step "cargo test ${EXTRA_ARGS[*]}"
cargo test "${EXTRA_ARGS[@]}"
print_success "tests pass"
