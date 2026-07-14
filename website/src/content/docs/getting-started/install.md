---
title: Install Quon
description: Install Quon's native toolchain, build the compiler, and prepare Qiskit Aer.
---

Quon compiles from source with Rust and LLVM/MLIR 22. Contributors should use **Devbox** for the native toolchain. Qiskit Aer provides the local simulator used in the quickstart.

## Prerequisites

You need:

- **Rust stable** via [rustup](https://rustup.rs/) — Quon uses the toolchain selected by `rust-toolchain.toml` (kept outside Devbox).
- **Devbox** (recommended) — provides LLVM/MLIR 22, libz3, Python 3.12, and Node 22 from a locked Nix environment.
- **Python 3.10 or newer** — used by the Qiskit Aer bridge (`devbox run setup-python` creates a venv).

Start by cloning the repository and entering it:

```bash
git clone https://github.com/arniber21/quon.git
cd quon
```

All remaining commands on this page run from the repository root.

## Contributor setup (Devbox)

Install [Devbox](https://www.jetify.com/devbox/docs/installing_devbox/) if you do not already have it (Nix is installed on first use):

```bash
curl -fsSL https://get.jetify.com/devbox | bash
```

Enter the project shell (or allow direnv; this repo commits a `.envrc`):

```bash
devbox shell
# optional: direnv allow
```

Confirm LLVM 22 and build:

```bash
llvm-config --version   # expect 22.x
cargo build --release
./target/release/quonc --version
```

`devbox.json` sets `MLIR_SYS_220_PREFIX` from `llvm-config --prefix` via a local flake (`nix/llvm-mlir`) that joins Nix's separate LLVM and MLIR packages into one Melior-compatible prefix.

Useful scripts: `devbox run build`, `devbox run test`, `devbox run check`, `devbox run setup-python`.

## Install Rust

If `rustup` is not installed, use its official installer:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

The repository selects stable Rust automatically. Confirm:

```bash
rustc --version
```

## Install the Aer bridge

Inside a Devbox shell (or any Python 3.10+):

```bash
devbox run setup-python
source .venv/bin/activate
python -c "from qiskit_aer import AerSimulator; print(AerSimulator())"
```

You are ready to [compile and simulate a Bell pair](/getting-started/quickstart/).

## Building from source (manual)

Prefer Devbox for day-to-day work. The steps below remain as an advanced fallback if you install LLVM/MLIR and Z3 yourself.

### macOS (Homebrew)

Install Apple's command-line tools if they are not already present:

```bash
xcode-select --install
```

```bash
brew install llvm@22 z3 python
export MLIR_SYS_220_PREFIX="$(brew --prefix llvm@22)"
export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
```

Add the same exports to `~/.zshrc` (or your shell's profile) so later terminal sessions keep the configuration.

### Ubuntu and Debian

```bash
sudo apt update
sudo apt install -y \
  build-essential curl wget gnupg lsb-release software-properties-common \
  libz3-dev python3 python3-pip python3-venv
```

Use the official [apt.llvm.org](https://apt.llvm.org/) installer, then install the LLVM 22 development packages:

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
```

Then `cargo build --release` as usual. `llvm-config --version` should report version 22.

## Troubleshooting

### Cargo cannot find `llvm-config`

If you use Devbox, ensure you are inside `devbox shell` (or direnv has loaded `.envrc`):

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
