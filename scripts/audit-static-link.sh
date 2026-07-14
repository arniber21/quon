#!/usr/bin/env bash
# Link audit gate for self-contained release binaries (issue #234 / Phase C).
#
# Pass criteria (plan defaults — not fully-static musl):
#   Linux:  ldd must not list libMLIR, libMLIR-C, or libz3
#           (glibc / libstdc++ / libgcc are OK)
#   Darwin: otool -L must not list Homebrew or Nix store paths for
#           LLVM / MLIR / Z3 (libSystem / libc++ are OK)
#
# Usage: ./scripts/audit-static-link.sh [bin ...]
# Default bins: target/release/{quonc,quonfmt,quon_lsp,quonlint}
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BINS=("$@")
if [[ ${#BINS[@]} -eq 0 ]]; then
  BINS=(
    target/release/quonc
    target/release/quonfmt
    target/release/quon_lsp
    target/release/quonlint
  )
fi

OS="$(uname -s)"
FAILED=0

fail() {
  echo "audit-static-link: FAIL: $*" >&2
  FAILED=1
}

audit_linux() {
  local bin="$1"
  local deps
  deps="$(ldd "$bin" 2>&1 || true)"
  echo "---- ldd $bin ----"
  echo "$deps"
  if echo "$deps" | grep -Eiq 'libMLIR([^[:alnum:]_-]|$)|libMLIR-C|libz3'; then
    fail "$bin links shared MLIR and/or z3 (see ldd above)"
  fi
  # Binaries built inside devbox can silently link /nix/store .so paths
  # (zlib, libxml2, ...) that won't exist on user machines.
  if echo "$deps" | grep -q '/nix/store/'; then
    fail "$bin links Nix store libraries (see ldd above)"
  fi
}

audit_darwin() {
  local bin="$1"
  local deps
  deps="$(otool -L "$bin" 2>&1 || true)"
  echo "---- otool -L $bin ----"
  echo "$deps"
  # No Homebrew / Nix store runtime deps (libSystem, libc++, libz, libxml2 OK).
  if echo "$deps" | grep -Eiq '/opt/homebrew/|/usr/local/(opt|Cellar)/|/nix/store/'; then
    fail "$bin has Homebrew or Nix store paths in otool -L (need system libs only)"
  fi
  if echo "$deps" | grep -Eiq 'libz3\.(dylib|so)|libMLIR'; then
    fail "$bin links libz3 or libMLIR as a shared library"
  fi
}

for bin in "${BINS[@]}"; do
  if [[ ! -x "$bin" ]]; then
    fail "missing or not executable: $bin"
    continue
  fi
  case "$OS" in
    Linux) audit_linux "$bin" ;;
    Darwin) audit_darwin "$bin" ;;
    *)
      fail "unsupported OS for link audit: $OS"
      ;;
  esac
done

if [[ "$FAILED" -ne 0 ]]; then
  echo "audit-static-link: FAILED" >&2
  exit 1
fi

echo "audit-static-link: OK ($OS)"
