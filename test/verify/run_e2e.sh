#!/usr/bin/env bash
# Run Python Aer end-to-end verification scripts (Phase 6+).
#
# Usage:
#   ./test/verify/run_e2e.sh          # all scripts
#   ./test/verify/run_e2e.sh bell     # single case by stem

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERIFY="$ROOT/test/verify"
QUONC="${QUONC:-$ROOT/target/release/quonc}"

run_one() {
  local stem="$1"
  local qn="$VERIFY/${stem}.qn"
  local py="$VERIFY/${stem}.py"
  if [[ ! -f "$qn" ]]; then
    echo "skip $stem (no .qn fixture)" >&2
    return 0
  fi
  if [[ ! -f "$py" ]]; then
    echo "skip $stem (no .py verifier)" >&2
    return 0
  fi
  echo "==> e2e $stem"
  python3 "$py" "$qn"
}

if [[ $# -gt 0 ]]; then
  for stem in "$@"; do
    run_one "$stem"
  done
else
  shopt -s nullglob
  for py in "$VERIFY"/*.py; do
    run_one "$(basename "$py" .py)"
  done
fi
