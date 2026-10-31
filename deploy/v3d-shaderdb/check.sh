#!/bin/bash
# Compile every pipeline with the Pi 5's Mesa (shaderdb.sh) and check it
# against baseline.txt: fails if any shader lost threads or started spilling.
#
#   check.sh [checkout]            check (the checkout defaults to this repo)
#   check.sh [checkout] --update   accept the new table as the baseline
#
# About a minute, no Pi needed. The Pi's own `V3D_DEBUG=shaderdb` printed the
# same numbers for every shader when this was checked (2026-10-31).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "${1:-$here/../..}" && pwd)
table=$(mktemp -t v3d-check.XXXXXX)
"$here/shaderdb.sh" "$repo" "$table.raw" >"$table" 2>/dev/null || true
if [[ "${2:-}" == "--update" ]]; then
    python3 "$here/check.py" "$here/baseline.txt" "$table" || true
    cp "$table" "$here/baseline.txt"
    echo "baseline updated"
else
    python3 "$here/check.py" "$here/baseline.txt" "$table"
fi
