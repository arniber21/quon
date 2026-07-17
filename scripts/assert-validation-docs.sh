#!/usr/bin/env bash
# Fail if agent validation docs regress to known-stale CI claims (issue #203).
# Positive anchors track the Justfile as orchestrator source of truth (ADR-0012).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VALIDATION="$ROOT/docs/agents/validation.md"
CODE_QUALITY="$ROOT/docs/agents/code-quality.md"
README="$ROOT/README.md"
NA_FT_DEMO="$ROOT/website/src/content/docs/guides/na-ft-demo.mdx"
FAILED=0

fail() {
  echo "assert-validation-docs: $*" >&2
  FAILED=1
}

# Phrases that were true historically and must not return.
if grep -q 'Not in CI yet: `lit test/lit/`' "$VALIDATION"; then
  fail "validation.md still claims lit is not in CI"
fi
if grep -q 'Aer verification for Bell/teleport/BV' "$VALIDATION"; then
  fail "validation.md still claims Aer covers only Bell/teleport/BV"
fi
if grep -q 'FileCheck IR tests (not in CI yet)' "$CODE_QUALITY"; then
  fail "code-quality.md still claims lit is not in CI"
fi
if grep -q 'tooling-check.sh' "$VALIDATION" "$CODE_QUALITY"; then
  fail "docs still reference deleted scripts/tooling-check.sh — use just ci-tooling"
fi

# Paths that moved (DepthExpr's canonical home is quon_core, not mlir_bridge)
# and must not be cited as if they still exist.
for stale_path in \
  'mlir_bridge/tests/depth_props.rs' \
  'mlir_bridge/src/dialect/depth.rs' \
  'mlir_bridge/fuzz/fuzz_targets/fuzz_depth_parse.rs' \
  '`frontend/src/typecheck.rs`'
do
  if grep -qF "$stale_path" "$VALIDATION" "$CODE_QUALITY"; then
    fail "a doc cites stale path: $stale_path"
  fi
done

# Positive anchors that must stay present (adapter of Justfile / workflow reality).
for needle in \
  'just test-ci' \
  'just doctor' \
  'QUON_REQUIRE_LIT' \
  'quonc/tests/lit.rs' \
  'test/verify/{bell,teleport,bernstein_vazirani,routing,grover,qft,ising,qaoa,shor}.py' \
  'cargo llvm-cov' \
  'taskless.yml' \
  'flux.yml' \
  'release.yml' \
  '#180'
do
  if ! grep -qF "$needle" "$VALIDATION"; then
    fail "validation.md missing required anchor: $needle"
  fi
done

if ! grep -qF 'quon_core' "$CODE_QUALITY"; then
  fail "code-quality.md must mention quon_core as DepthExpr home"
fi
if ! grep -qF 'frontend/src/typecheck/mod.rs' "$CODE_QUALITY"; then
  fail "code-quality.md must cite the typecheck module (frontend/src/typecheck/mod.rs)"
fi
if ! grep -qF 'just test-ci' "$CODE_QUALITY"; then
  fail "code-quality.md must mention just test-ci as the pre-PR gate"
fi

# Neutral-atom FT compiler demo page (#279) must exist and stay linked from the
# README so cold-outreach reviewers can find the end-to-end path.
if [[ ! -f "$NA_FT_DEMO" ]]; then
  fail "missing neutral-atom FT demo page: website/src/content/docs/guides/na-ft-demo.mdx (#279)"
else
  for needle in \
    'surface_d3_cx.qn' \
    '--emit-qec-experiment' \
    '--emit-resource-report' \
    'devbox run' \
    'analytic' \
    'sampled'
  do
    if ! grep -qF -e "$needle" "$NA_FT_DEMO"; then
      fail "na-ft-demo.mdx missing required anchor: $needle"
    fi
  done
fi
if ! grep -qF '/guides/na-ft-demo' "$README"; then
  fail "README must link the neutral-atom FT demo page (/guides/na-ft-demo) (#279)"
fi

if [[ "$FAILED" -ne 0 ]]; then
  echo "assert-validation-docs: FAILED — update docs/agents to match Justfile + .github/workflows/" >&2
  exit 1
fi

echo "assert-validation-docs: OK"
