#!/usr/bin/env bash
# Build a .deb that installs Quon CLIs into /usr/bin (issue #235 / Phase D).
#
# Expects release binaries already present (run scripts/release.sh first), or
# pass paths via QUON_BIN_DIR. Uses dpkg-deb (Debian/Ubuntu) — no nfpm required.
#
# Usage (from repo root):
#   ./scripts/release.sh && ./scripts/package-deb.sh
#   QUON_RELEASE_VERSION=v0.1.0 ./scripts/package-deb.sh
#
# Output:
#   dist/quon_${version}_${deb_arch}.deb
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "package-deb: dpkg-deb not found (Debian/Ubuntu packaging only)" >&2
  exit 1
fi

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
# Debian versions cannot start with a letter; keep git describe as-is if numeric.
DEB_VERSION="$VERSION_STRIPPED"

ARCH_HOST="$(uname -m)"
case "$ARCH_HOST" in
  x86_64|amd64) DEB_ARCH="amd64" ;;
  aarch64|arm64) DEB_ARCH="arm64" ;;
  *)
    echo "package-deb: unsupported arch: $ARCH_HOST" >&2
    exit 1
    ;;
esac

BIN_DIR="${QUON_BIN_DIR:-$ROOT/target/release}"
BINS=(quonc quonfmt quon_lsp quonlint)
for b in "${BINS[@]}"; do
  if [[ ! -x "$BIN_DIR/$b" ]]; then
    echo "package-deb: missing binary $BIN_DIR/$b (run scripts/release.sh first)" >&2
    exit 1
  fi
done

# Optional: re-run link audit so a .deb cannot ship shared MLIR/z3.
if [[ -x "$ROOT/scripts/audit-static-link.sh" ]]; then
  "$ROOT/scripts/audit-static-link.sh" \
    "$BIN_DIR/quonc" \
    "$BIN_DIR/quonfmt" \
    "$BIN_DIR/quon_lsp" \
    "$BIN_DIR/quonlint"
fi

DIST_DIR="$ROOT/dist"
PKG_ROOT="$DIST_DIR/deb-root"
PKG_NAME="quon_${DEB_VERSION}_${DEB_ARCH}"
rm -rf "$PKG_ROOT"
mkdir -p "$PKG_ROOT/DEBIAN" "$PKG_ROOT/usr/bin"

for b in "${BINS[@]}"; do
  install -m 755 "$BIN_DIR/$b" "$PKG_ROOT/usr/bin/$b"
done

cat >"$PKG_ROOT/DEBIAN/control" <<EOF
Package: quon
Version: ${DEB_VERSION}
Section: devel
Priority: optional
Architecture: ${DEB_ARCH}
Maintainer: Quon maintainers <https://github.com/arniber21/quon>
Homepage: https://github.com/arniber21/quon
Depends: libc6
Description: Quon quantum compiler CLI tools
 Self-contained Quon CLIs (quonc, quonfmt, quon_lsp, quonlint).
 No system LLVM, MLIR, or Z3 required at runtime — those are linked
 statically into the binaries. Optional Qiskit Aer simulation still
 needs Python deps from the Quon repository.
EOF

mkdir -p "$DIST_DIR"
DEB_PATH="$DIST_DIR/${PKG_NAME}.deb"
# root:root ownership inside the archive; -Zxz for wider dpkg compatibility.
dpkg-deb --root-owner-group -Zxz --build "$PKG_ROOT" "$DEB_PATH"

echo "package-deb: wrote $DEB_PATH"
ls -lh "$DEB_PATH"
echo "package-deb: install with: sudo apt install ./$(basename "$DEB_PATH")"
