#!/usr/bin/env bash

# Demo smoke — launch the offline demo with a short visual checklist.
# Invoked via `./dev demo-smoke`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "${SCRIPT_DIR}/lib.sh"
cd_to_project_root

print_info "Demo smoke checklist:"
cat << 'EOF'
  0-6s    Visible turn: inbound activity, tool bubble, assistant bubble.
  14-19s  Silent turn: green power-up sparkle is visible above Sebastian; no thought bubble.
  21-26s  Error path: anomaly bubble appears, then the sprite settles.
EOF

if [[ "${LOG_TARGETS:-console}" == "console" ]]; then
    export LOG_TARGETS=both
fi
export MODE=demo

print_step "./dev run --mode demo --log ${LOG_TARGETS}"
"${SCRIPT_DIR}/run.sh"
