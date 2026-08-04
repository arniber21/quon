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
  'ci-rustdoc'
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

# First-run validation setup must stay self-diagnosing (#373): the
# getting-started install page and CONTRIBUTING.md must both distinguish
# `devbox shell` from one-shot host-shell commands and keep a `devbox run --`
# alternative adjacent to every documented `just` recipe, so the two
# invocation paths cannot drift.
INSTALL="$ROOT/website/src/content/docs/getting-started/install.md"
CONTRIB="$ROOT/CONTRIBUTING.md"

install_needles=(
  'devbox shell'
  'devbox run -- just doctor'
  'devbox run -- just test-fast'
  'devbox run -- just test-ci'
  'devbox run -- just setup-python'
  'command not found: just'
  'First-run toolchain readiness'
)
for needle in "${install_needles[@]}"; do
  if ! grep -qF -e "$needle" "$INSTALL"; then
    fail "install.md missing two-path anchor: $needle (#373)"
  fi
done

contrib_needles=(
  'devbox shell'
  'devbox run -- just test-fast'
  'devbox run -- just test-ci'
  'devbox run -- just ci-rust'
  'devbox run -- just ci-tooling'
  'devbox run -- just ci-docs-assert'
  'command not found: just'
)
for needle in "${contrib_needles[@]}"; do
  if ! grep -qF -e "$needle" "$CONTRIB"; then
    fail "CONTRIBUTING.md missing two-path anchor: $needle (#373)"
  fi
done

# The one-shot and in-shell forms must stay in lockstep: every `just <recipe>`
# named in install.md's main-workflow table must also carry a
# `devbox run -- just <recipe>` row, and vice versa.
just_recipes_in_install=$(grep -oE 'just (doctor|test-fast|test-ci|setup-python)' "$INSTALL" | sort -u)
devbox_run_in_install=$(grep -oE 'devbox run -- just (doctor|test-fast|test-ci|setup-python)' "$INSTALL" | sed 's/devbox run -- //' | sort -u)
if [[ "$just_recipes_in_install" != "$devbox_run_in_install" ]]; then
  fail "install.md: just recipes and devbox run -- alternatives drifted (#373)\n  just: $(printf %s "$just_recipes_in_install" | tr '\n' ' ')\n  devbox: $(printf %s "$devbox_run_in_install" | tr '\n' ' ')"
fi

if [[ "$FAILED" -ne 0 ]]; then
  echo "assert-validation-docs: FAILED — update docs/agents to match Justfile + .github/workflows/" >&2
  exit 1
fi

echo "assert-validation-docs: OK"

# Documentation quality-gate corpus (issue #377): validate the declared docs
# corpus — executable examples, generated excerpts, internal links, commands.
python3 "$ROOT/scripts/assert-docs-corpus.py"
