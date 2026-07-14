#!/usr/bin/env bash
# Curl installer for Quon self-contained GitHub Release archives (issue #235).
#
# Picks the right asset for the host OS/arch, extracts, and installs into
# PREFIX (default: /usr/local). No LLVM/MLIR/Z3 required at runtime.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/arniber21/quon/main/scripts/install.sh | bash
#   curl -fsSL ... | bash -s -- --version 0.1.0
#   PREFIX="$HOME/.local" ./scripts/install.sh
#
# Env:
#   QUON_REPO      owner/name (default: arniber21/quon)
#   QUON_VERSION   version without leading v (default: latest release)
#   PREFIX         install prefix (default: /usr/local)
set -euo pipefail

REPO="${QUON_REPO:-arniber21/quon}"
VERSION="${QUON_VERSION:-}"
PREFIX="${PREFIX:-/usr/local}"
BIN_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version|-v)
      VERSION="${2:?}"
      shift 2
      ;;
    --prefix)
      PREFIX="${2:?}"
      shift 2
      ;;
    --repo)
      REPO="${2:?}"
      shift 2
      ;;
    -h|--help)
      sed -n '2,16p' "$0"
      exit 0
      ;;
    *)
      echo "install.sh: unknown arg: $1" >&2
      exit 1
      ;;
  esac
done

VERSION="${VERSION#v}"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "install.sh: missing required command: $1" >&2
    exit 1
  fi
}

need_cmd curl
need_cmd tar
need_cmd uname
need_cmd mktemp

OS_KERNEL="$(uname -s)"
ARCH_HOST="$(uname -m)"
case "$ARCH_HOST" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *)
    echo "install.sh: unsupported architecture: $ARCH_HOST" >&2
    exit 1
    ;;
esac

case "$OS_KERNEL" in
  Linux) OS_TRIPLE="unknown-linux-gnu" ;;
  Darwin) OS_TRIPLE="apple-darwin" ;;
  *)
    echo "install.sh: unsupported OS: $OS_KERNEL" >&2
    exit 1
    ;;
esac

api="https://api.github.com/repos/${REPO}/releases"
if [[ -z "$VERSION" ]]; then
  echo "install.sh: resolving latest release for ${REPO}..."
  tag="$(curl -fsSL "${api}/latest" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
  if [[ -z "$tag" ]]; then
    echo "install.sh: could not resolve latest release (no releases yet?)" >&2
    exit 1
  fi
  VERSION="${tag#v}"
  echo "install.sh: latest is v${VERSION}"
fi

asset="quon-${VERSION}-${ARCH}-${OS_TRIPLE}.tar.gz"
url="https://github.com/${REPO}/releases/download/v${VERSION}/${asset}"

tmpdir="$(mktemp -d)"
cleanup() { rm -rf "$tmpdir"; }
trap cleanup EXIT

echo "install.sh: downloading ${url}"
if ! curl -fsSL "$url" -o "$tmpdir/$asset"; then
  echo "install.sh: download failed — is v${VERSION} published for ${ARCH}-${OS_TRIPLE}?" >&2
  exit 1
fi

tar -xzf "$tmpdir/$asset" -C "$tmpdir"
# Archive layout: quon-${VERSION}-${ARCH}-${OS_TRIPLE}/{quonc,...}
extract_dir="$tmpdir/quon-${VERSION}-${ARCH}-${OS_TRIPLE}"
if [[ ! -d "$extract_dir" ]]; then
  extract_dir=""
  for candidate in "$tmpdir"/quon-*/; do
    if [[ -x "${candidate}quonc" ]]; then
      extract_dir="${candidate%/}"
      break
    fi
  done
fi
if [[ -z "$extract_dir" || ! -x "$extract_dir/quonc" ]]; then
  echo "install.sh: archive layout unexpected (no quonc)" >&2
  ls -laR "$tmpdir" >&2 || true
  exit 1
fi

BIN_DIR="${PREFIX}/bin"
mkdir -p "$BIN_DIR"

# Prefer install(1); fall back to cp + chmod.
install_bin() {
  local src="$1"
  local dst="$BIN_DIR/$(basename "$src")"
  if [[ -w "$BIN_DIR" ]] || [[ -w "$PREFIX" ]]; then
    if command -v install >/dev/null 2>&1; then
      install -m 755 "$src" "$dst"
    else
      cp "$src" "$dst"
      chmod 755 "$dst"
    fi
  else
    echo "install.sh: ${BIN_DIR} not writable — retry with sudo or PREFIX=\$HOME/.local" >&2
    if command -v install >/dev/null 2>&1; then
      sudo install -m 755 "$src" "$dst"
    else
      sudo cp "$src" "$dst"
      sudo chmod 755 "$dst"
    fi
  fi
}

for b in quonc quonfmt quon_lsp quonlint; do
  if [[ ! -x "$extract_dir/$b" ]]; then
    echo "install.sh: missing $b in archive" >&2
    exit 1
  fi
  install_bin "$extract_dir/$b"
  echo "install.sh: installed ${BIN_DIR}/$b"
done

echo "install.sh: done. Try: quonc --version"
echo "install.sh: no system LLVM/MLIR/Z3 required. Optional Aer: pip install -r python/requirements.txt"
