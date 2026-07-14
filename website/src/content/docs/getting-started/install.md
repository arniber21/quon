---
title: Install Quon
description: Set up Quon from source with the pinned compiler toolchain and optional verification dependencies.
---

Quon is a Rust workspace with native LLVM/MLIR and Z3 dependencies. Use Devbox
for the contributor setup: it pins LLVM/MLIR 22, libz3, Python, Node, and
`just`, while Rust itself is selected by `rust-toolchain.toml`.

## Recommended setup: Devbox

```bash
git clone https://github.com/arniber21/quon.git
cd quon
curl -fsSL https://get.jetify.com/devbox | bash   # if needed
devbox shell          # or: direnv allow
just doctor
cargo build -p quonc
```

`devbox.json` sets `MLIR_SYS_220_PREFIX` from `llvm-config --prefix` via the
local `nix/llvm-mlir` flake, so Melior sees a single LLVM/MLIR prefix.

The main local workflows are:

```bash
just doctor      # check required tools
just test-fast   # fast Rust-focused path
just test-ci     # local CI-parity path
```

## Optional: Qiskit Aer verification

The compiler CLIs do not require Python at runtime. The Aer verification seam
does:

```bash
just setup-python
source .venv/bin/activate
python -c "from qiskit_aer import AerSimulator; print(AerSimulator())"
```

With that environment active, the quickstart can compile Quon source and sample
the emitted OpenQASM 3 with Qiskit Aer.

## Manual source setup

Prefer Devbox for day-to-day work. If you manage the native dependencies
yourself, install LLVM/MLIR 22 and libz3, then set `MLIR_SYS_220_PREFIX` to the
LLVM installation root.

### macOS

```bash
xcode-select --install
brew install llvm@22 z3 python
export MLIR_SYS_220_PREFIX="$(brew --prefix llvm@22)"
export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
cargo build -p quonc
```

Add the exports to `~/.zshrc` or your shell profile for later sessions.

### Ubuntu and Debian

```bash
sudo apt update
sudo apt install -y \
  build-essential curl wget gnupg lsb-release software-properties-common \
  libz3-dev python3 python3-pip python3-venv
```

Use the official [apt.llvm.org](https://apt.llvm.org/) installer, then install
the LLVM 22 development packages:

```bash
wget -q https://apt.llvm.org/llvm.sh
chmod +x llvm.sh
sudo ./llvm.sh 22
sudo apt install -y \
  libmlir-22-dev mlir-22-tools llvm-22-dev llvm-22-tools libpolly-22-dev
rm llvm.sh
```

```bash
export MLIR_SYS_220_PREFIX=/usr/lib/llvm-22
export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
cargo build -p quonc
```

`llvm-config --version` should report version 22.

## Release packaging path

The repository contains release machinery for self-contained CLI artifacts:

- `scripts/install.sh` for curl-style installs from GitHub Releases.
- `scripts/package-deb.sh` for Debian packages.
- `scripts/generate-homebrew-formula.sh` for Homebrew formula generation.
- `scripts/release.sh` for static release assembly.
- `.github/workflows/release.yml` for tagged release builds.

That packaging path is part of Quon's production-tooling direction: the release
flow is designed around prebuilt `quonc`, `quonfmt`, `quon_lsp`, and
`quonlint` binaries, while contributors keep the reproducible Devbox source
build.

## Troubleshooting

### Cargo cannot find `llvm-config`

Check that the shell has loaded the Devbox or manual LLVM environment:

```bash
echo "$MLIR_SYS_220_PREFIX"
command -v llvm-config
llvm-config --version
```

The prefix must be the LLVM 22 installation root, not its `bin` directory.

### Python cannot import Qiskit Aer

Activate the virtual environment and install the checked-in requirements:

```bash
source .venv/bin/activate
python -m pip install -r python/requirements.txt
```

Then continue to the [quickstart](/getting-started/quickstart/).
