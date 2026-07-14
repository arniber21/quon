#!/usr/bin/env bash
# Self-contained release build for quonc / quonfmt / quon_lsp / quonlint (issue #234).
#
# Goal: ship binaries that need no runtime LLVM, MLIR, or Z3 on the user's machine.
# Fully static musl is out of scope; glibc/libstdc++ (Linux) and libSystem/libc++
# (Darwin) remain OK.
#
# Static linking knobs (document for Phase C / release CI):
#
#   MLIR / LLVM (mlir-sys 220):
#     - Prefers static when libMLIR*.a exist under $MLIR_SYS_220_PREFIX/lib
#       (Nix llvm-mlir flake and Homebrew llvm@22 both provide .a).
#     - Unset MLIR_SYS_LINK_SHARED (or set to 0). Setting it to 1 forces shared
#       libMLIR and will fail the link audit.
#     - MLIR_SYS_220_PREFIX must point at a Melior-compatible prefix
#       (Devbox init_hook sets this from `llvm-config --prefix`).
#
#   Z3 (z3 0.12 / z3-sys 0.8):
#     - Do NOT use Cargo feature `static-link-z3` — the z3 tree bundled in
#       z3-sys 0.8 fails to build on modern Apple Clang / CMake.
#     - Instead `./scripts/build-static-z3.sh` builds upstream Z3 (default
#       4.16.0, matching Devbox) into target/z3-static with only libz3.a.
#     - Env vars consumed by z3-sys + this script:
#         Z3_LIBRARY_PATH_OVERRIDE — $Z3_PREFIX/lib (static .a only)
#         Z3_SYS_Z3_HEADER         — $Z3_PREFIX/include/z3.h
#         LIBRARY_PATH             — prepend $Z3_PREFIX/lib
#         RUSTFLAGS                — adds -lc++ (Darwin) or -lstdc++ (Linux)
#                                    so the static archive's C++ deps resolve
#         Z3_VERSION / Z3_PREFIX   — optional overrides for build-static-z3.sh
#     - There is no Z3_SYS_STATIC env var in z3-sys 0.8.
#
# Usage (from repo root, inside `devbox shell` or with MLIR_SYS_220_PREFIX set):
#   ./scripts/release.sh
#   # or: devbox run release
#
# Outputs:
#   target/release/{quonc,quonfmt,quon_lsp,quonlint}
#   dist/quon-{version}-{arch}-{os}.tar.gz
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ -z "${MLIR_SYS_220_PREFIX:-}" ]]; then
  if command -v llvm-config >/dev/null 2>&1; then
    export MLIR_SYS_220_PREFIX="$(llvm-config --prefix)"
  else
    echo "release: MLIR_SYS_220_PREFIX is unset and llvm-config is not on PATH" >&2
    echo "release: enter \`devbox shell\` (or set the prefix) and retry" >&2
    exit 1
  fi
fi

# Refuse shared MLIR for release artifacts.
unset MLIR_SYS_LINK_SHARED || true

# Require static MLIR archives in the prefix (mlir-sys falls back to shared otherwise).
libdir="$MLIR_SYS_220_PREFIX/lib"
shopt -s nullglob
mlir_archives=("$libdir"/libMLIR*.a)
shopt -u nullglob
if [[ ${#mlir_archives[@]} -eq 0 ]]; then
  echo "release: no libMLIR*.a under $libdir — cannot produce static MLIR link" >&2
  exit 1
fi

./scripts/build-static-z3.sh
Z3_PREFIX="${Z3_PREFIX:-$ROOT/target/z3-static}"
export Z3_LIBRARY_PATH_OVERRIDE="$Z3_PREFIX/lib"
export Z3_SYS_Z3_HEADER="$Z3_PREFIX/include/z3.h"
export LIBRARY_PATH="$Z3_PREFIX/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"

OS_KERNEL="$(uname -s)"
case "$OS_KERNEL" in
  Darwin) CXXLIB="c++" ;;
  Linux) CXXLIB="stdc++" ;;
  *)
    echo "release: unsupported OS: $OS_KERNEL" >&2
    exit 1
    ;;
esac

# Static libz3.a needs the C++ standard library at final link time.
RUSTFLAGS_EXTRA=(
  "-Lnative=${Z3_PREFIX}/lib"
  "-Clink-arg=-l${CXXLIB}"
)

# Stage static-only system libs so mlir-sys's `-lzstd` (etc.) resolves to .a
# instead of Homebrew/Nix dylibs. macOS ld prefers .dylib in the same directory.
SYSROOT_STATIC="$ROOT/target/sysroot-static"
rm -rf "$SYSROOT_STATIC"
mkdir -p "$SYSROOT_STATIC/lib"

find_static_lib() {
  local name="$1"
  local dir
  local search_dirs=(
    "$MLIR_SYS_220_PREFIX/lib"
    "${LIBRARY_PATH:-}"
  )
  if command -v brew >/dev/null 2>&1; then
    local brew_prefix
    brew_prefix="$(brew --prefix "$name" 2>/dev/null || true)"
    if [[ -n "$brew_prefix" ]]; then
      search_dirs+=("$brew_prefix/lib")
    fi
  fi
  local expanded=()
  for dir in "${search_dirs[@]}"; do
    if [[ -z "$dir" ]]; then
      continue
    fi
    if [[ "$dir" == *:* ]]; then
      IFS=':' read -r -a parts <<<"$dir"
      expanded+=("${parts[@]}")
    else
      expanded+=("$dir")
    fi
  done
  for dir in "${expanded[@]}"; do
    if [[ -f "$dir/lib${name}.a" ]]; then
      echo "$dir/lib${name}.a"
      return 0
    fi
  done
  return 1
}

for syslib in zstd z3; do
  if archive="$(find_static_lib "$syslib")"; then
    echo "release: staging static $archive"
    cp "$archive" "$SYSROOT_STATIC/lib/lib${syslib}.a"
    RUSTFLAGS_EXTRA+=("-lstatic=${syslib}")
  else
    # z3 is required (built above); zstd is best-effort for LLVM system deps.
    if [[ "$syslib" == "z3" ]]; then
      # Prefer the release-built static prefix.
      if [[ -f "$Z3_PREFIX/lib/libz3.a" ]]; then
        echo "release: staging static $Z3_PREFIX/lib/libz3.a"
        cp "$Z3_PREFIX/lib/libz3.a" "$SYSROOT_STATIC/lib/libz3.a"
        RUSTFLAGS_EXTRA+=("-lstatic=z3")
      else
        echo "release: missing libz3.a" >&2
        exit 1
      fi
    else
      echo "release: warning: no lib${syslib}.a found; dynamic ${syslib} may fail the link audit" >&2
    fi
  fi
done

RUSTFLAGS_EXTRA=(
  "-Lnative=${SYSROOT_STATIC}/lib"
  "${RUSTFLAGS_EXTRA[@]}"
)
case "$OS_KERNEL" in
  Darwin)
    RUSTFLAGS_EXTRA+=("-Clink-arg=-Wl,-search_paths_first")
    ;;
esac

export RUSTFLAGS="${RUSTFLAGS:-} ${RUSTFLAGS_EXTRA[*]}"

VERSION="${QUON_RELEASE_VERSION:-}"
if [[ -z "$VERSION" ]]; then
  if git describe --tags --exact-match HEAD >/dev/null 2>&1; then
    VERSION="$(git describe --tags --exact-match HEAD)"
  elif git describe --tags --always >/dev/null 2>&1; then
    VERSION="$(git describe --tags --always)"
  else
    VERSION="0.0.0-dev"
  fi
fi
VERSION_STRIPPED="${VERSION#v}"

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
esac

case "$OS_KERNEL" in
  Linux) OS_TRIPLE="unknown-linux-gnu" ;;
  Darwin) OS_TRIPLE="apple-darwin" ;;
esac

echo "release: MLIR_SYS_220_PREFIX=$MLIR_SYS_220_PREFIX"
echo "release: Z3_PREFIX=$Z3_PREFIX"
echo "release: building version=$VERSION target=${ARCH}-${OS_TRIPLE}"

# Force rebuild of native-link crates against the static sysroot.
cargo clean -p z3-sys -p mlir-sys 2>/dev/null || true

cargo build --release \
  -p quonc -p quonfmt -p quon_lsp -p quonlint-cli

BINS=(quonc quonfmt quon_lsp quonlint)
for b in "${BINS[@]}"; do
  if [[ ! -x "target/release/$b" ]]; then
    echo "release: missing binary target/release/$b" >&2
    exit 1
  fi
done

# Strip debug symbols when strip(1) is available (large static LLVM/MLIR blobs).
if command -v strip >/dev/null 2>&1; then
  for b in "${BINS[@]}"; do
    strip -S "target/release/$b" 2>/dev/null || strip "target/release/$b" || true
  done
fi

./scripts/audit-static-link.sh \
  target/release/quonc \
  target/release/quonfmt \
  target/release/quon_lsp \
  target/release/quonlint

DIST_DIR="$ROOT/dist"
STAGE="$DIST_DIR/stage"
ARCHIVE_NAME="quon-${VERSION_STRIPPED}-${ARCH}-${OS_TRIPLE}"
rm -rf "$STAGE"
mkdir -p "$STAGE/$ARCHIVE_NAME"

for b in "${BINS[@]}"; do
  cp "target/release/$b" "$STAGE/$ARCHIVE_NAME/"
done

cat >"$STAGE/$ARCHIVE_NAME/INSTALL.txt" <<EOF
Quon ${VERSION} (${ARCH}-${OS_TRIPLE})

Self-contained CLI binaries — no system LLVM, MLIR, or Z3 required at runtime.

Contents:
  quonc      Quon compiler
  quonfmt    Formatter
  quon_lsp   Language server
  quonlint   Linter

Install (example):
  tar -xzf ${ARCHIVE_NAME}.tar.gz
  sudo install -m 755 ${ARCHIVE_NAME}/quonc ${ARCHIVE_NAME}/quonfmt \\
    ${ARCHIVE_NAME}/quon_lsp ${ARCHIVE_NAME}/quonlint /usr/local/bin/

Optional Qiskit Aer simulation still needs Python deps from the Quon repo
(\`pip install -r python/requirements.txt\`); the CLIs themselves do not.

Built with static MLIR/LLVM (mlir-sys) and a release-built static libz3.a
(see scripts/build-static-z3.sh / scripts/release.sh).
EOF

mkdir -p "$DIST_DIR"
ARCHIVE_PATH="$DIST_DIR/${ARCHIVE_NAME}.tar.gz"
tar -C "$STAGE" -czf "$ARCHIVE_PATH" "$ARCHIVE_NAME"

echo "release: wrote $ARCHIVE_PATH"
ls -lh "$ARCHIVE_PATH"
for b in "${BINS[@]}"; do
  ls -lh "target/release/$b"
done
