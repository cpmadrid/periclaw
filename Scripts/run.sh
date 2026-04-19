#!/usr/bin/env bash

# Run — cargo run wrapper with OpenClaw mode selection.
# Invoked via `./dev run [--release] [--mode mock|ws]`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "${SCRIPT_DIR}/lib.sh"
cd_to_project_root

BUILD_TYPE="${BUILD_TYPE:-debug}"
MODE="${MODE:-auto}"        # auto | mock | ws
NO_BUILD="${NO_BUILD:-false}"
LOG_TARGETS="${LOG_TARGETS:-console}"   # console | file | both
LOG_FILE="${LOG_FILE:-}"                # empty → auto-generate under Logs/
RUST_LOG_DEFAULT="${RUST_LOG_DEFAULT:-sassy_mc=debug,warn}"
export RUST_LOG="${RUST_LOG:-$RUST_LOG_DEFAULT}"

require_bin cargo "Install Rust from https://rustup.rs"

# Secrets come from Doppler (sassy-dog standard). If the user already has
# OPENCLAW_TOKEN exported, respect it; otherwise prepend `doppler run --`
# so the app inherits secrets from the monorepo's doppler config.
# Doppler is opt-in: if it's not installed or not configured for this
# repo, we fall through silently and the app's built-in token bootstrap
# (keychain → plaintext fallback → ~/.openclaw/openclaw.json) takes over.
doppler_prefix=()
if [[ -z "${OPENCLAW_TOKEN:-}" ]] \
   && command -v doppler >/dev/null 2>&1 \
   && doppler secrets get OPENCLAW_TOKEN --plain >/dev/null 2>&1; then
    doppler_prefix=(doppler run --)
    print_info "Secrets: doppler run (project=$(doppler configure get project --plain 2>/dev/null || echo '?'))"
fi

# Resolve mode.
case "$MODE" in
    mock)
        export OPENCLAW_MOCK=1
        mode_desc="mock (fixture scenario)"
        ;;
    ws)
        unset OPENCLAW_MOCK
        mode_desc="native ws"
        ;;
    auto)
        if [[ -n "${OPENCLAW_MOCK:-}" ]]; then
            mode_desc="mock (inherited)"
        else
            mode_desc="native ws"
        fi
        ;;
    *)
        print_error "unknown --mode '$MODE' (expected: mock, ws, auto)"
        exit 1
        ;;
esac

print_info "Mode: ${mode_desc}"
print_info "RUST_LOG=${RUST_LOG}"

# Resolve log destination if we're writing to a file.
if [[ "$LOG_TARGETS" == "file" || "$LOG_TARGETS" == "both" ]]; then
    if [[ -z "$LOG_FILE" ]]; then
        mkdir -p "${PROJECT_ROOT}/Logs"
        LOG_FILE="${PROJECT_ROOT}/Logs/desktop-$(date +%Y%m%d-%H%M%S).log"
    else
        mkdir -p "$(dirname "$LOG_FILE")"
    fi
    print_info "Log file: ${LOG_FILE}"
fi

cargo_args=(run)
if [[ "$BUILD_TYPE" == "release" ]]; then
    cargo_args+=(--release)
fi

# Decide what to launch (cargo run vs. pre-built binary) as an array
# so redirection can wrap it uniformly below.
if [[ "$NO_BUILD" == "true" ]]; then
    case "$BUILD_TYPE" in
        release) bin_path="${RELEASE_BUILD_DIR}/${APP_EXECUTABLE}" ;;
        *)       bin_path="${DEBUG_BUILD_DIR}/${APP_EXECUTABLE}"   ;;
    esac
    if [[ ! -x "$bin_path" ]]; then
        print_error "no binary at ${bin_path} — run ./dev build first"
        exit 1
    fi
    launch=("${doppler_prefix[@]}" "$bin_path")
else
    launch=("${doppler_prefix[@]}" cargo "${cargo_args[@]}")
fi
print_step "${launch[*]}"

# Route output. `exec` keeps the parent PID so Ctrl-C still lands on the
# app. For `both` we pipe through tee and then exit with the upstream's
# status — no exec since pipelines can't be execed cleanly.
case "$LOG_TARGETS" in
    console)
        exec "${launch[@]}"
        ;;
    file)
        exec "${launch[@]}" > "$LOG_FILE" 2>&1
        ;;
    both)
        set +e
        "${launch[@]}" 2>&1 | tee "$LOG_FILE"
        exit "${PIPESTATUS[0]}"
        ;;
esac
