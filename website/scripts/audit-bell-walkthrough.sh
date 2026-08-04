#!/usr/bin/env bash
#
# audit-bell-walkthrough.sh — prevent full-copy drift of the shared Bell example.
#
# The Bell circuit (H @0 |> CNOT @(0, 1)) appears across several docs pages as a
# motivating example.  Issue #384 assigns each occurrence a single, non-overlapping
# role so the early site does not feel repetitive:
#
#   Home (/)                            teaser + link
#   Quickstart (/getting-started/...)   execution + artifact inspection
#   Language (/language/introduction/)  progressive concept explanation
#   Cookbook (/cookbook/bell/)          verified algorithm/artifact analysis
#
# This script checks that the *full* Bell walkthrough (the `run` block that
# executes the circuit) does not leak into pages whose role is teaser-only,
# and that the canonical owners still carry it.  Run it from the repo root or
# the website root.
#
# Usage:  bash website/scripts/audit-bell-walkthrough.sh
# Exit:   0 = pass, 1 = drift detected

set -euo pipefail

# Resolve the docs directory relative to this script.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCS_DIR="$SCRIPT_DIR/../src/content/docs"

if [ ! -d "$DOCS_DIR" ]; then
  echo "FAIL: docs directory not found at $DOCS_DIR" >&2
  exit 1
fi

# --- Canonical owners of the full Bell execution block -----------------------
#
# These pages are *allowed* to show the full `fn main(): Q<(Bit, Bit)> = run {`
# block that executes bell_state.  Pages not in this list must not paste it.
CANONICAL_OWNERS=(
  "getting-started/quickstart.md"
  "language/introduction.md"
  "cookbook/bell.mdx"
)

# --- Concept-illustration pages (grandfathered) ------------------------------
#
# These pages reference bell_state() @ qreg(2) inside a *different* concept
# discussion (linearity, monad, qubit registers, design philosophy).  They are
# not full walkthroughs and are allowed to use the example, but must not grow
# into a full copy (compile commands, MLIR, Aer output).  New pages are NOT
# added here automatically — add one only if it illustrates a distinct concept.
CONCEPT_ILLUSTRATIONS=(
  "language/qubits.md"
  "language/linearity.md"
  "language/monad.md"
  "why-quon/philosophy.md"
)

# --- Pages that must stay teaser-only ----------------------------------------
#
# Home is a teaser: it may show the circuit *definition* but must NOT show the
# execution `run` block, MLIR excerpts, or Aer output.
TEASER_ONLY=(
  "index.mdx"
)

errors=0

# The signature of the full Bell execution walkthrough: the run block that
# binds bell_state() to qreg(2) and measures both qubits.
RUN_BLOCK_PATTERN='bell_state.*@ qreg(2)'

# Pages whose role permits the Bell run block (canonical + grandfathered).
ALLOWED_RUN_BLOCK=( "${CANONICAL_OWNERS[@]}" "${CONCEPT_ILLUSTRATIONS[@]}" )

echo "Checking Bell walkthrough ownership across docs…"

# 1.  Teaser-only pages must not contain the full run block.
for page in "${TEASER_ONLY[@]}"; do
  file="$DOCS_DIR/$page"
  if [ ! -f "$file" ]; then
    echo "  SKIP  $page (not found)"
    continue
  fi
  if grep -q "$RUN_BLOCK_PATTERN" "$file"; then
    echo "  FAIL  $page contains the Bell execution run block — teaser-only page."
    echo "        Remove the full walkthrough and link to a canonical owner instead."
    errors=$((errors + 1))
  else
    echo "  OK    $page is teaser-only (no execution run block)."
  fi
done

# 2.  Canonical owners must still carry the full walkthrough.
for page in "${CANONICAL_OWNERS[@]}"; do
  file="$DOCS_DIR/$page"
  if [ ! -f "$file" ]; then
    echo "  SKIP  $page (not found)"
    continue
  fi
  if grep -q "$RUN_BLOCK_PATTERN" "$file"; then
    echo "  OK    $page carries the Bell execution block (canonical owner)."
  else
    echo "  FAIL  $page lost the Bell execution block — canonical owner must keep it."
    errors=$((errors + 1))
  fi
done

# 3.  No *other* docs page should paste the full Bell run block.
#     (The circuit *definition* alone is fine — many pages reference it.)
while IFS= read -r file; do
  rel="${file#$DOCS_DIR/}"
  # Skip all allowed pages (canonical + concept illustrations + teaser).
  skip=0
  for p in "${ALLOWED_RUN_BLOCK[@]}" "${TEASER_ONLY[@]}"; do
    if [ "$rel" = "$p" ]; then skip=1; break; fi
  done
  [ "$skip" -eq 1 ] && continue
  if grep -q "$RUN_BLOCK_PATTERN" "$file"; then
    echo "  FAIL  $rel contains the Bell execution run block."
    echo "        Reduce to an excerpt and link to a canonical owner:"
    echo "          /getting-started/quickstart/  (execution)"
    echo "          /language/introduction/       (concepts)"
    echo "          /cookbook/bell/               (verified analysis)"
    errors=$((errors + 1))
  fi
done < <(grep -rl --include='*.md' --include='*.mdx' 'bell_state' "$DOCS_DIR" 2>/dev/null || true)

# 4.  Concept-illustration pages must not grow into full walkthroughs.
#     They may use the run block but must NOT add compile commands, Aer
#     output, or MLIR excerpts (the cookbook's `quantum.circ.func @bell_state`
#     schematic).  The bare op name `quantum.circ.func` is allowed because it
#     appears legitimately in architecture/ADR discussions.
WALKTHROUGH_MARKERS=(
  '--emit-qasm'
  'quon_aer.py'
  '--dump-ir'
  'quantum.circ.func @bell_state'
)
for page in "${CONCEPT_ILLUSTRATIONS[@]}"; do
  file="$DOCS_DIR/$page"
  if [ ! -f "$file" ]; then
    continue
  fi
  for marker in "${WALKTHROUGH_MARKERS[@]}"; do
    if grep -qF -- "$marker" "$file"; then
      echo "  FAIL  $page contains '$marker' — concept page grew into a full walkthrough."
      echo "        Move execution/analysis material to a canonical owner and link."
      errors=$((errors + 1))
    fi
  done
  echo "  OK    $page uses Bell as illustration (no walkthrough drift)."
done

echo ""
if [ "$errors" -eq 0 ]; then
  echo "PASS: Bell walkthrough ownership is clean."
  exit 0
else
  echo "FAIL: $errors ownership violation(s) detected."
  echo "      See issue #384 for the role assignment."
  exit 1
fi
