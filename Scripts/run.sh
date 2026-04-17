#!/usr/bin/env bash

# Run — cargo run wrapper with OpenClaw mode selection.
# Invoked via `./dev run [--release] [--mode mock|ssh|ws]`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "${SCRIPT_DIR}/lib.sh"
cd_to_project_root

BUILD_TYPE="${BUILD_TYPE:-debug}"
MODE="${MODE:-auto}"        # auto | mock | ssh | ws
SSH_HOST="${SSH_HOST:-workstation}"
NO_BUILD="${NO_BUILD:-false}"
RUST_LOG_DEFAULT="${RUST_LOG_DEFAULT:-sassy_mc=debug,warn}"
export RUST_LOG="${RUST_LOG:-$RUST_LOG_DEFAULT}"

require_bin cargo "Install Rust from https://rustup.rs"

# Resolve mode.
case "$MODE" in
    mock)
        export OPENCLAW_MOCK=1
        unset OPENCLAW_SSH_HOST
        mode_desc="mock (fixture scenario)"
        ;;
    ssh)
        unset OPENCLAW_MOCK
        export OPENCLAW_SSH_HOST="$SSH_HOST"
        mode_desc="ssh (${SSH_HOST})"
        ;;
    ws)
        unset OPENCLAW_MOCK OPENCLAW_SSH_HOST
        mode_desc="native ws (scoped methods need pairing — M3.3)"
        ;;
    auto)
        # Use whatever the environment already has; don't override.
        if [[ -n "${OPENCLAW_MOCK:-}" ]]; then
            mode_desc="mock (inherited)"
        elif [[ -n "${OPENCLAW_SSH_HOST:-}" ]]; then
            mode_desc="ssh (inherited ${OPENCLAW_SSH_HOST})"
        else
            mode_desc="native ws (no env vars set)"
        fi
        ;;
    *)
        print_error "unknown --mode '$MODE' (expected: mock, ssh, ws, auto)"
        exit 1
        ;;
esac

print_info "Mode: ${mode_desc}"
print_info "RUST_LOG=${RUST_LOG}"

cargo_args=(run)
if [[ "$BUILD_TYPE" == "release" ]]; then
    cargo_args+=(--release)
fi

if [[ "$NO_BUILD" == "true" ]]; then
    # Skip cargo run; exec the pre-built binary directly.
    case "$BUILD_TYPE" in
        release) bin_path="${RELEASE_BUILD_DIR}/${APP_EXECUTABLE}" ;;
        *)       bin_path="${DEBUG_BUILD_DIR}/${APP_EXECUTABLE}"   ;;
    esac
    if [[ ! -x "$bin_path" ]]; then
        print_error "no binary at ${bin_path} — run ./dev build first"
        exit 1
    fi
    print_step "exec ${bin_path}"
    exec "$bin_path"
fi

print_step "cargo ${cargo_args[*]}"
exec cargo "${cargo_args[@]}"
