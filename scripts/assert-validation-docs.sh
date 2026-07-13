#!/usr/bin/env bash
# Fail if agent validation docs regress to known-stale CI claims (issue #203).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VALIDATION="$ROOT/docs/agents/validation.md"
CODE_QUALITY="$ROOT/docs/agents/code-quality.md"
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

# Positive anchors that must stay present (adapter of ci.yml reality).
for needle in \
  'quonc/tests/lit.rs' \
  'test/verify/{bell,teleport,bernstein_vazirani,routing,grover,qft,ising,qaoa,shor}.py' \
  'cargo llvm-cov' \
  'taskless.yml' \
  'flux.yml' \
  '#180'
do
  if ! grep -qF "$needle" "$VALIDATION"; then
    fail "validation.md missing required anchor: $needle"
  fi
done

if ! grep -qF 'quon_core' "$CODE_QUALITY"; then
  fail "code-quality.md must mention quon_core as DepthExpr home"
fi

if [[ "$FAILED" -ne 0 ]]; then
  echo "assert-validation-docs: FAILED — update docs/agents to match .github/workflows/" >&2
  exit 1
fi

echo "assert-validation-docs: OK"
