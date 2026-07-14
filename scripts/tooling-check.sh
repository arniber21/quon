#!/usr/bin/env bash
# Tooling quality gates (quonfmt, quonlint, LSP smoke) — mirrors CI tooling job.
#
# Usage:
#   ./scripts/tooling-check.sh              # full local gate (fmt + lint + LSP)
#   ./scripts/tooling-check.sh --ci         # exact CI corpus + thresholds
#   ./scripts/tooling-check.sh --fmt-only
#   ./scripts/tooling-check.sh --lint-only
#   ./scripts/tooling-check.sh --lsp-only
#   ./scripts/tooling-check.sh --full       # all .qn fixtures (slow)
#
# Prerequisites: same as cargo test (LLVM 22 + MLIR + z3 on PATH).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CI_CORPUS="$ROOT/test/tooling/ci-corpus.txt"
QUONLINT_CONFIG="$ROOT/.quonlint.toml"
RELEASE=(--release)
BUILD_PKGS=(-p quonfmt -p quonlint-cli -p quon_lsp)

MODE=local
RUN_FMT=1
RUN_LINT=1
RUN_LSP=1
CORPUS_FILES=()

usage() {
  sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ci)
      MODE=ci
      shift
      ;;
    --fmt-only)
      RUN_FMT=1
      RUN_LINT=0
      RUN_LSP=0
      shift
      ;;
    --lint-only)
      RUN_FMT=0
      RUN_LINT=1
      RUN_LSP=0
      shift
      ;;
    --lsp-only)
      RUN_FMT=0
      RUN_LINT=0
      RUN_LSP=1
      shift
      ;;
    --full)
      MODE=full
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

read_corpus() {
  local manifest="$1"
  if [[ ! -f "$manifest" ]]; then
    echo "error: corpus manifest not found: $manifest" >&2
    exit 2
  fi
  local path
  while IFS= read -r path || [[ -n "$path" ]]; do
    path="${path%%#*}"
    path="$(echo "$path" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
    [[ -z "$path" ]] && continue
    if [[ ! -f "$ROOT/$path" ]]; then
      echo "error: corpus path missing: $path" >&2
      exit 2
    fi
    if [[ "$path" != *.qn ]]; then
      echo "error: corpus path is not .qn: $path" >&2
      exit 2
    fi
    CORPUS_FILES+=("$path")
  done < "$manifest"
}

collect_full_corpus() {
  local path
  while IFS= read -r path; do
    CORPUS_FILES+=("$path")
  done < <(find frontend/tests/fixtures -name '*.qn' -type f | sort)
  if [[ -d test/verify ]]; then
    while IFS= read -r path; do
      CORPUS_FILES+=("$path")
    done < <(find test/verify -maxdepth 1 -name '*.qn' -type f | sort)
  fi
}

if [[ "$MODE" == "full" ]]; then
  collect_full_corpus
else
  read_corpus "$CI_CORPUS"
fi

if [[ ${#CORPUS_FILES[@]} -eq 0 ]]; then
  echo "error: no corpus files selected" >&2
  exit 2
fi

echo "tooling-check: mode=$MODE files=${#CORPUS_FILES[@]}"

# Reuse binaries only if they still run: a CI-cache-restored binary can
# reference nix store libraries that no longer exist on this machine.
if [[ -x "$ROOT/target/release/quonfmt" && -x "$ROOT/target/release/quonlint" ]] \
  && "$ROOT/target/release/quonfmt" --version >/dev/null 2>&1 \
  && "$ROOT/target/release/quonlint" --version >/dev/null 2>&1; then
  echo "tooling-check: reusing existing release binaries"
else
  cargo build "${RELEASE[@]}" "${BUILD_PKGS[@]}"
fi

QUONFMT="$ROOT/target/release/quonfmt"
QUONLINT="$ROOT/target/release/quonlint"

if [[ ! -x "$QUONFMT" || ! -x "$QUONLINT" ]]; then
  echo "error: tooling binaries missing after build" >&2
  exit 2
fi

if [[ "$RUN_FMT" -eq 1 ]]; then
  echo "==> quonfmt --check (${#CORPUS_FILES[@]} files)"
  "$QUONFMT" --check "${CORPUS_FILES[@]}"
fi

if [[ "$RUN_LINT" -eq 1 ]]; then
  echo "==> quonlint --config .quonlint.toml --fail-on error"
  "$QUONLINT" \
    --config "$QUONLINT_CONFIG" \
    --fail-on error \
    "${CORPUS_FILES[@]}"
fi

if [[ "$RUN_LSP" -eq 1 ]]; then
  echo "==> cargo test -p quon_lsp --test smoke --include-ignored"
  cargo test "${RELEASE[@]}" -p quon_lsp --test smoke -- --include-ignored
fi

echo "tooling-check: OK"
