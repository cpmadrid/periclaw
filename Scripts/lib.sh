#!/usr/bin/env bash

# Common Library Functions — Mission Control Desktop
# Sourced by ./dev and the activity scripts in Scripts/.
# Keep this a thin wrapper; activity-specific logic belongs in its own script.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
export PROJECT_ROOT

# Source configuration
# shellcheck source=./config.sh
source "${SCRIPT_DIR}/config.sh"

cd_to_project_root() {
    cd "$PROJECT_ROOT" || exit 1
}

# Colored output — matches lupita's print_* palette.
print_info()    { echo -e "${BLUE}${INFO}${NC}  $1"; }
print_success() { echo -e "${GREEN}${SUCCESS}${NC} $1"; }
print_error()   { echo -e "${RED}${ERROR}${NC} $1" >&2; }
print_warning() { echo -e "${YELLOW}⚠️${NC}  $1"; }
print_step()    { echo -e "${BLUE}${BUILD}${NC}  $1"; }

# Require a binary on PATH; abort with a useful message if missing.
require_bin() {
    local bin="$1"
    local hint="${2:-}"
    if ! command -v "$bin" >/dev/null 2>&1; then
        print_error "$bin not found on PATH"
        [[ -n "$hint" ]] && print_info "$hint"
        exit 1
    fi
}

# Get build number via Scripts/get-build-number.sh (consistent with CI).
get_build_number() {
    "${SCRIPT_DIR}/get-build-number.sh"
}

# Get current git commit hash (short) — used in --version output.
git_short_sha() {
    if git rev-parse --git-dir >/dev/null 2>&1; then
        git rev-parse --short HEAD
    else
        echo "no-git"
    fi
}
