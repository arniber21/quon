---
title: Install Quon
description: Install Quon's native toolchain, build the compiler, and prepare Qiskit Aer.
---

Quon compiles from source with Rust and LLVM/MLIR 22. Qiskit Aer provides the local simulator used in the quickstart.

## Prerequisites

You need:

- **Rust stable** — Quon uses the toolchain selected by `rust-toolchain.toml`.
- **LLVM and MLIR 22** — required by Melior 0.27, Quon's Rust interface to MLIR. Cargo installs Melior from the workspace dependency.
- **libz3** — used by the frontend's refinement checker.
- **Python 3.10 or newer** — used by the Qiskit Aer bridge.

Start by cloning the repository and entering it:

```bash
git clone https://github.com/arniber21/quon.git
cd quon
```

All remaining commands on this page run from the repository root.

## macOS

Install Apple's command-line tools if they are not already present:

```bash
xcode-select --install
```

Install the native dependencies with [Homebrew](https://brew.sh/):

```bash
brew install llvm@22 z3 python
```

LLVM is a versioned Homebrew formula, so tell Melior where to find it:

```bash
export MLIR_SYS_220_PREFIX="$(brew --prefix llvm@22)"
export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
```

Add the same exports to `~/.zshrc` (or your shell's profile) so later terminal sessions keep the configuration.

## Ubuntu and Debian

Install the base build, Python, and Z3 packages:

```bash
sudo apt update
sudo apt install -y \
  build-essential curl wget gnupg lsb-release software-properties-common \
  libz3-dev python3 python3-pip python3-venv
```

Use the official [apt.llvm.org](https://apt.llvm.org/) installer, then install the LLVM 22 development packages used by Quon:

```bash
wget -q https://apt.llvm.org/llvm.sh
chmod +x llvm.sh
sudo ./llvm.sh 22
sudo apt install -y \
  libmlir-22-dev mlir-22-tools llvm-22-dev llvm-22-tools libpolly-22-dev
rm llvm.sh
```

Point Melior at that installation:

```bash
export MLIR_SYS_220_PREFIX=/usr/lib/llvm-22
export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
```

Add the exports to `~/.bashrc` (or your shell's profile) to persist them.

## Install Rust

If `rustup` is not installed, use its official installer:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

The repository selects stable Rust automatically. Confirm that Rust and LLVM 22 are visible:

```bash
rustc --version
llvm-config --version
```

`llvm-config --version` should report version 22.

## Build the compiler

Build the optimized `quonc` binary:

```bash
cargo build --release
./target/release/quonc --version
```

The compiler is now available at `./target/release/quonc`.

## Install the Aer bridge

Keep Python packages isolated in a virtual environment:

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install -r python/requirements.txt
```

The requirements include Qiskit, Qiskit Aer, and the OpenQASM 3 importer. Verify the simulator imports:

```bash
python -c "from qiskit_aer import AerSimulator; print(AerSimulator())"
```

You are ready to [compile and simulate a Bell pair](/getting-started/quickstart/).

## Troubleshooting

### Cargo cannot find `llvm-config`

Check that the prefix and executable path are active in the current shell:

```bash
echo "$MLIR_SYS_220_PREFIX"
command -v llvm-config
llvm-config --version
```

The prefix must be the LLVM 22 installation root, not its `bin` directory.

### Python cannot import OpenQASM 3

Activate the virtual environment and install the complete checked-in requirements:

```bash
source .venv/bin/activate
python -m pip install -r python/requirements.txt
```
