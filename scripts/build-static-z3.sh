#!/usr/bin/env bash
# Build a static libz3.a prefix for release linking (issue #234).
#
# z3 0.12 / z3-sys 0.8's Cargo feature `static-link-z3` vendors an old Z3 tree
# that fails to compile on modern Apple Clang / CMake. Instead we build a
# current Z3 (matching Devbox's pin) once into target/z3-static and point
# z3-sys at it via Z3_LIBRARY_PATH_OVERRIDE + Z3_SYS_Z3_HEADER.
#
# Usage: ./scripts/build-static-z3.sh
# Env:
#   Z3_VERSION   — default 4.16.0 (Devbox pin)
#   Z3_PREFIX    — install prefix (default: <repo>/target/z3-static)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

Z3_VERSION="${Z3_VERSION:-4.16.0}"
Z3_PREFIX="${Z3_PREFIX:-$ROOT/target/z3-static}"
SRC_DIR="$ROOT/target/z3-src/z3-${Z3_VERSION}"
BUILD_DIR="$ROOT/target/z3-build"

if [[ -f "$Z3_PREFIX/lib/libz3.a" && -f "$Z3_PREFIX/include/z3.h" ]]; then
  echo "build-static-z3: reusing $Z3_PREFIX"
  exit 0
fi

if ! command -v cmake >/dev/null 2>&1; then
  echo "build-static-z3: cmake is required on PATH" >&2
  exit 1
fi

mkdir -p "$ROOT/target/z3-src"
if [[ ! -d "$SRC_DIR" ]]; then
  echo "build-static-z3: fetching Z3 ${Z3_VERSION}"
  curl -fsSL \
    "https://github.com/Z3Prover/z3/archive/refs/tags/z3-${Z3_VERSION}.tar.gz" \
    | tar -xz -C "$ROOT/target/z3-src"
  # Upstream tarball extracts to z3-z3-VERSION/
  if [[ -d "$ROOT/target/z3-src/z3-z3-${Z3_VERSION}" && ! -d "$SRC_DIR" ]]; then
    mv "$ROOT/target/z3-src/z3-z3-${Z3_VERSION}" "$SRC_DIR"
  fi
fi

if [[ ! -d "$SRC_DIR" ]]; then
  echo "build-static-z3: missing source tree at $SRC_DIR" >&2
  exit 1
fi

echo "build-static-z3: configuring → $Z3_PREFIX"
rm -rf "$BUILD_DIR"
cmake -S "$SRC_DIR" -B "$BUILD_DIR" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$Z3_PREFIX" \
  -DZ3_BUILD_LIBZ3_SHARED=FALSE \
  -DZ3_BUILD_EXECUTABLE=FALSE \
  -DZ3_BUILD_TEST_EXECUTABLES=FALSE \
  -DCMAKE_POLICY_VERSION_MINIMUM=3.5

cmake --build "$BUILD_DIR" --parallel "${CMAKE_BUILD_PARALLEL_LEVEL:-$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)}"
cmake --install "$BUILD_DIR"

# Ensure the prefix only exposes the static archive (no accidental .dylib/.so).
rm -f "$Z3_PREFIX/lib"/libz3.dylib "$Z3_PREFIX/lib"/libz3.*.dylib \
  "$Z3_PREFIX/lib"/libz3.so "$Z3_PREFIX/lib"/libz3.so.*

if [[ ! -f "$Z3_PREFIX/lib/libz3.a" ]]; then
  echo "build-static-z3: expected $Z3_PREFIX/lib/libz3.a" >&2
  exit 1
fi

echo "build-static-z3: OK → $Z3_PREFIX/lib/libz3.a"
