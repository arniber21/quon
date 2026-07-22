#!/usr/bin/env bash
# Refresh the checked-in viz showcase goldens under samples/visualization/goldens/
# (issue #189). Every showcase entry below is a documented `quonc` invocation
# against an existing, linked-not-forked `.qn` fixture (test/verify/, test/na/)
# — this script is the single source of truth for how each golden was produced.
#
# Usage:
#   samples/visualization/refresh_goldens.sh            # regenerate goldens in place
#   samples/visualization/refresh_goldens.sh --check    # regenerate to a scratch dir
#                                                        # and diff against committed
#                                                        # goldens; exits non-zero on
#                                                        # any drift (for CI wiring)
#
# `quonc/tests/viz_showcase.rs` already re-runs these same invocations in-process
# and byte-compares against the committed goldens on every `cargo test`, so
# `--check` here is a convenience mirror for humans, not a second CI gate.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

if ! command -v jq >/dev/null 2>&1; then
  echo "refresh_goldens: jq is required to extract the stable 'metrics' object from" >&2
  echo "  --metrics-json output. It is not a devbox.json package — install it from" >&2
  echo "  your OS package manager (e.g. \`brew install jq\` / \`apt-get install jq\`;" >&2
  echo "  GitHub Actions' ubuntu-latest runners already carry it) and re-run." >&2
  exit 1
fi

QUONC="${QUONC_BIN:-$ROOT/target/debug/quonc}"
if [[ ! -x "$QUONC" ]]; then
  echo "refresh_goldens: building quonc (debug) — set QUONC_BIN to reuse an existing binary" >&2
  cargo build -p quonc --bin quonc
fi

NA_TARGET="targets/neutral_atom/generic_rna_v0.json"
IBM_TARGET="targets/ibm/fake_manila_v2.json"

MODE="write"
OUT_ROOT="samples/visualization/goldens"
if [[ "${1:-}" == "--check" ]]; then
  MODE="check"
  OUT_ROOT="$(mktemp -d)"
  trap 'rm -rf "$OUT_ROOT"' EXIT
fi

mkdir -p \
  "$OUT_ROOT/dense_swap_mismatch" \
  "$OUT_ROOT/teleport_dynamic" \
  "$OUT_ROOT/qft_depth" \
  "$OUT_ROOT/na_interaction_graph" \
  "$OUT_ROOT/na_schedule_metrics" \
  "$OUT_ROOT/noise_target_overlay"

# 1. Dense SWAP mismatch (#135) — K3 all-to-all QAOA cost layer mapped onto
# fake_manila_v2's 5-qubit line; the (0,2) interaction is not adjacent, so
# SABRE inserts a 3-CNOT SWAP network around it (visible in the QASM below).
# Note the `swap_count` metric stays 0 — it only counts a literal `SWAP` op,
# which has already been decomposed to CNOTs by the time metrics run. #135's
# mapper visualizer must recognize the 3-CNOT pattern directly, not rely on
# this counter.
"$QUONC" --target "$IBM_TARGET" --emit-qasm -q test/verify/qaoa.qn \
  > "$OUT_ROOT/dense_swap_mismatch/qaoa_manila.qasm"
"$QUONC" --target "$IBM_TARGET" --metrics-json - -q test/verify/qaoa.qn 2>/dev/null \
  | jq '{metrics}' > "$OUT_ROOT/dense_swap_mismatch/metrics.json"

# 2. Teleport / dynamic (#134) — by default, `measurement_deferral` (SPEC
# §7.1) collapses teleportation's classically-controlled corrections into
# coherent CNOT/CZ gates: the *source* has explicit `if z_bit then .. else ..`
# branches, but the compiled QASM has none. #134's multi-stage viewer needs
# to show that source-level control flow can vanish by the time it reaches
# a fixed target.
"$QUONC" --emit-qasm -q test/verify/teleport.qn \
  > "$OUT_ROOT/teleport_dynamic/teleport.qasm"
"$QUONC" --metrics-json - -q test/verify/teleport.qn 2>/dev/null \
  | jq '{metrics}' > "$OUT_ROOT/teleport_dynamic/metrics.json"

# 3. QFT depth (#134) — `qft_roundtrip = qft(3) |> adjoint(qft(3))` starts as
# a full recursive QFT + inverse (see "before_optimization.mlir"); circ-pass
# fixpoint (gate_cancellation/rotation_merging) proves the whole thing is the
# identity and erases it down to a bare pass-through (see
# "after_optimization.mlir"). This is the depth-reduction story a pass-diff
# visualizer should render.
DUMP="$(mktemp)"
"$QUONC" --emit-qasm --dump-ir -q test/verify/qft.qn > /dev/null 2> "$DUMP"
awk '/^--- after lowering ---$/{flag=1; next} /^--- after circ passes ---$/{flag=0} flag' \
  "$DUMP" > "$OUT_ROOT/qft_depth/before_optimization.mlir"
awk '/^--- after circ passes ---$/{flag=1; next} /^--- after dynamic passes ---$/{flag=0} flag' \
  "$DUMP" > "$OUT_ROOT/qft_depth/after_optimization.mlir"
rm -f "$DUMP"
"$QUONC" --metrics-json - -q test/verify/qft.qn 2>/dev/null \
  | jq '{metrics}' > "$OUT_ROOT/qft_depth/metrics.json"

# 4. NA interaction graph (#113, #136) — QAOA MaxCut-style dense interaction
# graph as a Graphviz DOT export; feeds both the closed #113 script and the
# open #136 topology canvas.
"$QUONC" --target "$NA_TARGET" --na-backend zoned --emit-na-graph - -q \
  test/na/qaoa_graph.qn > "$OUT_ROOT/na_interaction_graph/qaoa_graph.dot"

# 5. NA bell/QAOA schedule (#113, #136) — analytic resource reports (not the
# full `--emit-na-schedule` envelope, which enumerates every trap site in the
# target's zones — ~8,437 AtomSites / ~1 MB of JSON for even `bell.qn` on
# generic_rna_v0 — small enough to check in on its own, but not something we
# want to diff on every schedule-layout change). These are the small,
# deterministic schedule-metrics summaries #113's script and #136's canvas
# both consume.
"$QUONC" --target "$NA_TARGET" --na-backend zoned --emit-resource-report - -q --verify-na \
  test/na/bell.qn > "$OUT_ROOT/na_schedule_metrics/bell_zoned.resource_report.json"
"$QUONC" --target "$NA_TARGET" --na-backend zoned --emit-resource-report - -q --verify-na \
  test/na/qaoa_graph.qn > "$OUT_ROOT/na_schedule_metrics/qaoa_graph_zoned.resource_report.json"

# 6. Noise-aware target overlay (#136) — `ising.qn` is already
# nearest-neighbor, so it maps onto fake_manila_v2's line with zero SWAPs;
# the interesting per-edge data for a topology overlay is the *target's*
# checked-in `noise.two_qubit_fidelity` / `noise.readout_error` fields
# (targets/ibm/fake_manila_v2.json), not anything new this script emits.
# Contrast with entry 5's NA `error_budget`, which is per-mechanism
# (rydberg/movement/transfer/...), not per-edge — #136 needs to reconcile
# both noise vocabularies onto one canvas.
"$QUONC" --target "$IBM_TARGET" --emit-qasm -q test/verify/ising.qn \
  > "$OUT_ROOT/noise_target_overlay/ising_manila.qasm"
"$QUONC" --target "$IBM_TARGET" --metrics-json - -q test/verify/ising.qn 2>/dev/null \
  | jq '{metrics}' > "$OUT_ROOT/noise_target_overlay/metrics.json"

if [[ "$MODE" == "check" ]]; then
  # Normalize floats to 4 decimal places in JSON goldens before diffing:
  # `estimated_fidelity` and `gate_fidelity_product` are analytic float
  # computations that vary across platforms (arm64 vs x86_64 FP units).
  # Non-JSON files (QASM, DOT, MLIR) pass through unchanged.
  normalize() {
    if [[ "$1" == *.json ]]; then
      jq '(.. | numbers) |= ((. * 1e4 | round) / 1e4)' "$1" 2>/dev/null || cat "$1"
    else
      cat "$1"
    fi
  }
  diff_ok=true
  while IFS= read -r -d '' golden; do
    rel="${golden#samples/visualization/goldens/}"
    regenerated="$OUT_ROOT/$rel"
    if [[ ! -f "$regenerated" ]]; then
      echo "refresh_goldens --check: missing regenerated golden: $rel" >&2
      diff_ok=false
      continue
    fi
    if ! diff <(normalize "$golden") <(normalize "$regenerated") >/dev/null 2>&1; then
      echo "refresh_goldens --check: committed golden $rel is STALE — regenerate with samples/visualization/refresh_goldens.sh" >&2
      diff "$golden" "$regenerated" >&2 || true
      diff_ok=false
    fi
  done < <(find samples/visualization/goldens -type f -print0)
  if $diff_ok; then
    echo "refresh_goldens --check: committed goldens are up to date"
  else
    exit 1
  fi
else
  echo "refresh_goldens: wrote samples/visualization/goldens/"
fi
