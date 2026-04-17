#!/usr/bin/env bash

# Build — cargo build wrapper with release/debug toggle + optional strip.
# Invoked via `./dev build [--release]`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "${SCRIPT_DIR}/lib.sh"
cd_to_project_root

BUILD_TYPE="${BUILD_TYPE:-debug}"
CLEAN="${CLEAN:-false}"

require_bin cargo "Install Rust from https://rustup.rs"

if [[ "$CLEAN" == "true" ]]; then
    print_step "cargo clean"
    cargo clean
fi

BUILD_NUMBER="$(get_build_number)"
export BUILD_NUMBER
export APP_BUILD_NUMBER="$BUILD_NUMBER"
export APP_COMMIT="$(git_short_sha)"

print_step "Building $APP_NAME v${APP_VERSION} (#${BUILD_NUMBER}, ${APP_COMMIT}) — ${BUILD_TYPE}"

if [[ "$BUILD_TYPE" == "release" ]]; then
    cargo build --release
    bin_path="${RELEASE_BUILD_DIR}/${APP_EXECUTABLE}"
    print_step "strip ${bin_path}"
    if command -v strip >/dev/null 2>&1; then
        strip "$bin_path" 2>/dev/null || true
    fi
    size="$(du -h "$bin_path" 2>/dev/null | cut -f1 || echo '?')"
    print_success "Built ${bin_path} (${size})"
else
    cargo build
    bin_path="${DEBUG_BUILD_DIR}/${APP_EXECUTABLE}"
    print_success "Built ${bin_path}"
fi
