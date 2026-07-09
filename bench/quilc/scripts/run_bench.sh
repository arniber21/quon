#!/usr/bin/env bash
# Run quonc vs quilc benchmark corpus (issue #116).
#
# Usage:
#   ./bench/quilc/scripts/run_bench.sh
#   QUONC=/path/to/quonc DOCKER=/usr/local/bin/docker ./bench/quilc/scripts/run_bench.sh
#   ./bench/quilc/scripts/run_bench.sh --skip-quilc   # quonc-only metrics
#
# Requires: quonc (built), Python 3, Docker with rigetti/quilc (unless --skip-quilc).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BENCH="$ROOT/bench/quilc"
OUT="${BENCH_OUT:-$BENCH/out}"
MANIFEST="$BENCH/corpus/manifest.json"
QUONC_TARGET="$BENCH/target/linear5_cx.json"
QUILC_ISA="$BENCH/target/linear5_cnot.qpu"

QUONC="${QUONC:-$ROOT/target/release/quonc}"
DOCKER="${DOCKER:-docker}"
QUILC_IMAGE="${QUILC_IMAGE:-rigetti/quilc:latest}"

SKIP_QUILC=0
for arg in "$@"; do
  case "$arg" in
    --skip-quilc) SKIP_QUILC=1 ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
  esac
done

mkdir -p "$OUT/quonc" "$OUT/quilc" "$OUT/qasm"

if [[ ! -x "$QUONC" ]]; then
  echo "error: quonc not found at $QUONC (build with: cargo build -p quonc --release)" >&2
  exit 1
fi

if [[ ! -f "$MANIFEST" ]]; then
  echo "error: missing manifest $MANIFEST" >&2
  exit 1
fi

quilc_available=0
if [[ "$SKIP_QUILC" -eq 0 ]]; then
  if command -v "$DOCKER" >/dev/null 2>&1 && "$DOCKER" image inspect "$QUILC_IMAGE" >/dev/null 2>&1; then
    quilc_available=1
  elif command -v "$DOCKER" >/dev/null 2>&1; then
    echo "info: pulling $QUILC_IMAGE ..." >&2
    if "$DOCKER" pull "$QUILC_IMAGE"; then
      quilc_available=1
    else
      echo "warning: could not pull $QUILC_IMAGE; continuing with --skip-quilc" >&2
      SKIP_QUILC=1
    fi
  else
    echo "warning: docker not found; continuing with --skip-quilc" >&2
    SKIP_QUILC=1
  fi
fi

run_quonc() {
  local id="$1"
  local qn="$2"
  local qasm_out="$OUT/qasm/${id}.qasm"
  local json_out="$OUT/quonc/${id}.json"
  local err_out="$OUT/quonc/${id}.err"
  if "$QUONC" --emit-qasm --metrics-json "$json_out" --target "$QUONC_TARGET" "$qn" \
      >"$qasm_out" 2>"$err_out"; then
    echo "quonc ok  $id"
  else
    echo "quonc FAIL $id (see $err_out)" >&2
    return 1
  fi
}

run_quilc() {
  local id="$1"
  local quil="$2"
  local out="$OUT/quilc/${id}.quil"
  local err="$OUT/quilc/${id}.err"
  # Mount bench/ so ISA path is stable inside the container.
  if cat "$quil" | "$DOCKER" run --rm -i \
      -v "$BENCH:/bench:ro" \
      "$QUILC_IMAGE" \
      -P --print-statistics --quiet \
      --isa /bench/target/linear5_cnot.qpu \
      >"$out" 2>"$err"; then
    echo "quilc ok  $id"
  else
    # quilc sometimes writes diagnostics to stdout; keep both
    echo "quilc FAIL $id (see $err / $out)" >&2
    return 1
  fi
}

echo "==> corpus from $MANIFEST"
IDS="$(python3 - <<PY
import json
m=json.load(open("$MANIFEST"))
for c in m["circuits"]:
    print(c["id"])
PY
)"

failed=0
for id in $IDS; do
  qn="$BENCH/corpus/qn/${id}.qn"
  quil="$BENCH/corpus/quil/${id}.quil"
  if [[ ! -f "$qn" ]]; then
    echo "missing $qn" >&2
    failed=1
    continue
  fi
  run_quonc "$id" "$qn" || failed=1
  if [[ "$SKIP_QUILC" -eq 0 && "$quilc_available" -eq 1 ]]; then
    if [[ ! -f "$quil" ]]; then
      echo "missing $quil" >&2
      failed=1
      continue
    fi
    run_quilc "$id" "$quil" || failed=1
  fi
done

echo "==> comparing metrics"
python3 "$BENCH/scripts/compare.py" \
  --manifest "$MANIFEST" \
  --out-dir "$OUT" \
  --skip-quilc "$SKIP_QUILC" \
  --table "$OUT/results.md" \
  --json "$OUT/results.json"

# Checked-in snapshot lives next to README (repo root .gitignore ignores **/out/).
cp "$OUT/results.md" "$BENCH/RESULTS.md"
cp "$OUT/results.json" "$BENCH/RESULTS.json"
echo "wrote $OUT/results.md and $BENCH/RESULTS.md"
echo "wrote $OUT/results.json and $BENCH/RESULTS.json"
if [[ "$failed" -ne 0 ]]; then
  echo "warning: some compiles failed; see $OUT/*/." >&2
  exit 1
fi
