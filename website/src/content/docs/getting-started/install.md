---
title: Install Quon
description: Install Quon CLIs for users, or set up the Devbox toolchain for contributors.
---

Quon ships **self-contained** CLI binaries (`quonc`, `quonfmt`, `quon_lsp`, `quonlint`).
End users do **not** need LLVM, MLIR, or Z3 on their machine. Contributors who build
from source should use **Devbox** for the native toolchain. Qiskit Aer remains optional
for local simulation.

## Users — prebuilt binaries (no LLVM required)

Pick one channel. All install the same statically linked Release artifacts from
[GitHub Releases](https://github.com/arniber21/quon/releases).

### Homebrew (macOS / Linuxbrew)

Once the tap is published (`arniber21/homebrew-quon`):

```bash
brew install arniber21/quon/quon
quonc --version
```

The formula downloads a Release bottle and has **no** runtime `depends_on "llvm@22"`
or `z3`. Until the tap exists, use the curl installer or a Release tarball below.
See [`packaging/homebrew/README.md`](https://github.com/arniber21/quon/blob/main/packaging/homebrew/README.md)
for tap publish steps.

### Debian / Ubuntu (`.deb`)

Download the `.deb` for your architecture from the latest
[GitHub Release](https://github.com/arniber21/quon/releases), then:

```bash
sudo apt install ./quon_*.deb
quonc --version
```

Binaries land in `/usr/bin`. Runtime needs only glibc / libstdc++ — not libMLIR or libz3.

### Curl installer

```bash
curl -fsSL https://raw.githubusercontent.com/arniber21/quon/main/scripts/install.sh | bash
# optional:
# curl -fsSL ... | bash -s -- --version 0.1.0
# PREFIX="$HOME/.local" bash scripts/install.sh
```

The script picks the right Release asset for your OS/arch and installs into
`PREFIX` (default `/usr/local`).

### GitHub Release tarball

```bash
# Example for Apple Silicon:
tar -xzf quon-*-aarch64-apple-darwin.tar.gz
sudo install -m 755 quon-*/quonc quon-*/quonfmt quon-*/quon_lsp quon-*/quonlint /usr/local/bin/
```

### Optional: Qiskit Aer simulation

The compiler CLIs do not require Python. For the Aer verify seam / quickstart
simulator, install Python deps from a clone (or copy `python/requirements.txt`):

```bash
python3 -m venv .venv && source .venv/bin/activate
pip install -r python/requirements.txt
```

You are ready to [compile and simulate a Bell pair](/getting-started/quickstart/).

---

## Contributors — Devbox

You need:

- **Rust stable** via [rustup](https://rustup.rs/) — Quon uses the toolchain selected by `rust-toolchain.toml` (kept outside Devbox).
- **Devbox** — LLVM/MLIR 22, libz3, Python 3.12, and Node 22 from a locked Nix environment.

```bash
git clone https://github.com/arniber21/quon.git
cd quon
curl -fsSL https://get.jetify.com/devbox | bash   # if needed
devbox shell          # or: direnv allow
llvm-config --version # expect 22.x
cargo build --release
./target/release/quonc --version
```

`devbox.json` sets `MLIR_SYS_220_PREFIX` from `llvm-config --prefix` via a local flake (`nix/llvm-mlir`) that joins Nix's separate LLVM and MLIR packages into one Melior-compatible prefix.

Useful scripts: `devbox run build`, `devbox run test`, `devbox run check`, `devbox run setup-python`, `devbox run release`.

Release packaging (`devbox run release` / `./scripts/release.sh`) builds self-contained
archives with static MLIR/LLVM and a release-built static `libz3.a`. Tag builds upload
tarballs, a Linux `.deb`, and a filled Homebrew `quon.rb` via `.github/workflows/release.yml`.

### Install Rust

If `rustup` is not installed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustc --version
```

### Install the Aer bridge

```bash
devbox run setup-python
source .venv/bin/activate
python -c "from qiskit_aer import AerSimulator; print(AerSimulator())"
```

## Building from source (manual)

Prefer Devbox for day-to-day work. The steps below remain as an advanced fallback if you install LLVM/MLIR and Z3 yourself.

### macOS (Homebrew)

```bash
xcode-select --install
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
