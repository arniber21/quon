#!/usr/bin/env bash
# LLVM source coverage for the stable workspace (excludes flux_verify).
#
# Prerequisites (once per machine):
#   rustup component add llvm-tools-preview
#   cargo install cargo-llvm-cov
#
# Usage:
#   ./scripts/coverage.sh              # summary table to stdout
#   ./scripts/coverage.sh --html       # HTML report in target/llvm-cov/html
#   ./scripts/coverage.sh --lcov       # lcov.info for CI upload tools
#
# Requires LLVM/MLIR on PATH (same as `cargo test` for mlir_bridge).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "error: cargo-llvm-cov not found. Install with: cargo install cargo-llvm-cov" >&2
  exit 1
fi

if ! rustup component list --installed | grep -q '^llvm-tools'; then
  echo "error: llvm-tools-preview not installed. Run: rustup component add llvm-tools-preview" >&2
  exit 1
fi

EXCLUDE=(--exclude flux_verify)

case "${1:-}" in
  --html)
    shift
    cargo llvm-cov "${EXCLUDE[@]}" --html --output-dir target/llvm-cov/html "$@"
    echo "HTML report: $ROOT/target/llvm-cov/html/index.html"
    ;;
  --lcov)
    shift
    cargo llvm-cov "${EXCLUDE[@]}" --lcov --output-path target/llvm-cov/lcov.info "$@"
    echo "lcov report: $ROOT/target/llvm-cov/lcov.info"
    ;;
  --help|-h)
    sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
    ;;
  *)
    cargo llvm-cov "${EXCLUDE[@]}" "$@"
    ;;
esac
