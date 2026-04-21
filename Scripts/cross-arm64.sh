#!/usr/bin/env bash

# Cross-compile for aarch64-unknown-linux-gnu.
# Uses `cross` if available; falls back to native cargo with the
# target flag (requires the toolchain + linker).
# Invoked via `./dev cross-arm64`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "${SCRIPT_DIR}/lib.sh"
cd_to_project_root

TARGET="aarch64-unknown-linux-gnu"

if command -v cross >/dev/null 2>&1; then
    print_step "cross build --release --target ${TARGET}"
    cross build --release --target "$TARGET"
else
    print_warning "\`cross\` not installed — falling back to cargo directly"
    print_info "Install with: cargo install cross --git https://github.com/cross-rs/cross"
    if ! rustup target list --installed | grep -q "^${TARGET}\$"; then
        print_step "rustup target add ${TARGET}"
        rustup target add "$TARGET"
    fi
    print_step "cargo build --release --target ${TARGET}"
    cargo build --release --target "$TARGET"
fi

bin_path="${BUILD_DIR}/${TARGET}/release/${APP_EXECUTABLE}"
if [[ -f "$bin_path" ]]; then
    size="$(du -h "$bin_path" | cut -f1)"
    print_success "Built ${bin_path} (${size})"
    print_info "Deploy with:  scp ${bin_path} workstation:/usr/local/bin/${APP_EXECUTABLE}"
fi
