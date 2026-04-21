#!/usr/bin/env bash

# Get Build Number — returns a monotonic build number for the app.
# Priority: $BUILD_NUMBER env var > git commit count.

set -euo pipefail

if [[ -n "${BUILD_NUMBER:-}" ]]; then
    echo "$BUILD_NUMBER"
    exit 0
fi

if command -v git >/dev/null 2>&1 && git rev-parse --git-dir >/dev/null 2>&1; then
    count="$(git rev-list --count HEAD 2>/dev/null || true)"
    if [[ -n "$count" ]]; then
        echo "$count"
        exit 0
    fi
fi

{
    echo "ERROR: Unable to determine build number"
    echo "Set the BUILD_NUMBER env var or run inside a git repo with history."
} >&2
exit 1
