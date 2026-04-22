#!/usr/bin/env bash

# Application Configuration — PeriClaw

# App information (APP_NAME is the user-visible display name; the
# crate/binary/config-dir stay lowercase per the project's naming
# convention — see memory/project_periclaw_naming.md).
APP_NAME="PeriClaw"
APP_EXECUTABLE="periclaw"
APP_CRATE="periclaw"

# Version — single source of truth is Cargo.toml.
# Read it lazily so bumping Cargo.toml is all you need.
if [[ -z "${APP_VERSION:-}" ]]; then
    if [[ -f "${PROJECT_ROOT:-.}/Cargo.toml" ]]; then
        APP_VERSION="$(
            awk '
                /^\[package\]/ { in_pkg = 1; next }
                /^\[/          { in_pkg = 0 }
                in_pkg && /^version[[:space:]]*=[[:space:]]*"/ {
                    match($0, /"[^"]*"/)
                    print substr($0, RSTART + 1, RLENGTH - 2)
                    exit
                }
            ' "${PROJECT_ROOT}/Cargo.toml"
        )"
    else
        APP_VERSION="0.0.0-unknown"
    fi
fi
export APP_VERSION

# Build directories (Cargo-controlled)
BUILD_DIR="target"
DEBUG_BUILD_DIR="${BUILD_DIR}/debug"
RELEASE_BUILD_DIR="${BUILD_DIR}/release"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Emoji indicators
SUCCESS="✅"
ERROR="❌"
INFO="ℹ️"
BUILD="🔨"
RUN="🚀"
WATCH="👁️"
TEST="🧪"
CLEAN="🧹"
LINT="🔍"
CROSS="🌍"
