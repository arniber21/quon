# Quon

An MLIR-based optimizing compiler for quantum programs. Accepts programs in the Quon language — a functional language with linear types and a monadic quantum interface — and emits OpenQASM 3.0 for execution on Qiskit Aer or real hardware.

## Prerequisites

| Dependency | Version | Notes |
|---|---|---|
| Rust | stable | via **rustup** (outside Devbox); see `rust-toolchain.toml` |
| Devbox + Nix | latest | recommended contributor toolchain — LLVM/MLIR 22, Z3, Python 3.12, Node 22 |
| Melior | **0.27.x** | Pinned in the workspace `Cargo.toml`; requires LLVM 22 |
| Python + Qiskit Aer | 3.10+ | Simulation verification (`test/verify/`); `devbox run setup-python` |
| Flux (optional) | nightly + z3 | Refinement types in `flux_verify`; install via [Flux install script](https://flux-rs.github.io/flux/guide/install.html) |

### Contributor setup (Devbox)

Install [Devbox](https://www.jetify.com/devbox/docs/installing_devbox/) (pulls in Nix on first use), then:

```bash
# rustup remains outside Devbox so rust-toolchain.toml is honored
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # if needed

devbox shell          # or: direnv allow  (committed .envrc)
llvm-config --version # expect 22.x
cargo build -p mlir_bridge -p quonc
# optional Aer bridge:
devbox run setup-python && source .venv/bin/activate
```

`devbox.json` / `devbox.lock` pin the native toolchain. A local flake at `nix/llvm-mlir` joins Nix's separate LLVM and MLIR store paths into one Melior-compatible prefix and sets `MLIR_SYS_220_PREFIX` in the shell `init_hook`.

Useful scripts: `devbox run build`, `devbox run test`, `devbox run check`.
Tag releases: `devbox run release` (static MLIR/LLVM + static libz3; see `scripts/release.sh`).

### Building from source (manual)

If you prefer not to use Devbox, install LLVM/MLIR 22 and libz3 yourself and set:

```bash
export MLIR_SYS_220_PREFIX=/path/to/llvm22   # e.g. /usr/lib/llvm-22 or $(brew --prefix llvm@22)
export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
```

On macOS with Homebrew: `brew install llvm@22 z3`, then point `MLIR_SYS_220_PREFIX` at `$(brew --prefix llvm@22)`.

On Ubuntu/Debian: use [apt.llvm.org](https://apt.llvm.org/) (`./llvm.sh 22`) and `apt install libz3-dev`.

## Build

```bash
cargo build --release
```

## Usage

```bash
# Compile to OpenQASM 3.0 (generic all-to-all target)
./target/release/quonc program.qn --emit-qasm

# Compile targeting a specific device
./target/release/quonc program.qn --target device.json --emit-qasm

# Neutral-atom schedule + resource report (#112)
./target/release/quonc test/na/bell.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule \
  --emit-resource-report

# Debug: dump IR stages / list pass pipeline
./target/release/quonc program.qn --dump-ir --verify-linear --emit-qasm
./target/release/quonc --list-passes

# Simulate with Qiskit Aer (via python/quon_aer.py — the verify seam)
# Prefer the bridge directly (it compiles + dialect-normalizes for Qiskit's importer):
QUONC=./target/release/quonc python python/quon_aer.py program.qn --shots 4096
# Or pipe QASM into the same bridge (still applies dialect normalize):
./target/release/quonc program.qn --emit-qasm | python python/quon_aer.py --shots 4096
# Unsupported: piping quonc QASM straight into qiskit.qasm3.loads without
# python/quon_aer.py — quonc emits spec-valid `bit[i] == 1` that the importer rejects.

# Experiment loop: live metrics and watch mode (see docs/agents/experiment-loop.md)
./target/release/quonc program.qn --watch --target device.json --metrics
```

## Compiler pipeline

`quonc` runs the fixed (gate-model) pipeline: circ fixpoint (`gate_cancellation`,
`rotation_merging`, `compiler_uncomputation`, `zx_simplification`) → monadic
lowering → dynamic passes → native-gate decomp → SABRE routing → depth
scheduling → OpenQASM 3.0. (`clifford_t_opt` is reserved for [#96](https://github.com/arniber21/quon/issues/96), not in the fixpoint — see #214.) Neutral-atom targets take a separate schedule/resource path (`--emit-na-schedule` / `--emit-resource-report`). Inspect stages with:

```bash
./target/release/quonc --list-passes
```

`--target device.json` selects connectivity, native gates, and noise for the fixed path; omitting it uses the built-in `generic_openqasm` all-to-all target. MVP story-by-story close-out: [`docs/plans/m5-closeout-audit.md`](docs/plans/m5-closeout-audit.md).

## Developer tooling

| Tool | Purpose |
|------|---------|
| `quon_lsp` | Language server (diagnostics, hover, completion, go-to-definition, semantic tokens) |
| `quonfmt` | Canonical Quon formatter (`quonfmt --check`, `-w`) — see [docs/quonfmt-style.md](docs/quonfmt-style.md) |
| `quonlint` | Algorithm-quality linter (`quonlint`, `quonlint.toml`) |
| VS Code | First-party extension: [`extensions/vscode-quon/`](extensions/vscode-quon/) (TextMate + LSP + `quonfmt`) |
| Neovim | First-party module: [`nvim-quon/`](nvim-quon/) (LSP + Tree-sitter + `quonfmt`) — see [`docs/agents/editor-setup.md`](docs/agents/editor-setup.md) |
| Zed | Dev extension: [`extensions/zed-quon/`](extensions/zed-quon/) (Tree-sitter + LSP + `quonfmt`) |
| Tree-sitter | Shared grammar for editors: [`tree-sitter-quon/`](tree-sitter-quon/) |

```bash
# Format check (CI corpus)
./target/release/quonfmt --check $(grep -v '^#' test/tooling/ci-corpus.txt)

# Lint
./target/release/quonlint --config .quonlint.toml --fail-on error $(grep -v '^#' test/tooling/ci-corpus.txt)

# All tooling gates (mirrors CI tooling job)
./scripts/tooling-check.sh --ci
```

See [docs/agents/validation.md](docs/agents/validation.md) for the full pre-PR checklist.

## Testing

```bash
cargo fmt --all -- --check   # formatting (also enforced in CI)
cargo clippy --workspace --exclude flux_verify --all-targets -- -D warnings
cargo build --examples --workspace --exclude flux_verify  # lit's FileCheck oracle binaries
cargo test --workspace --exclude flux_verify   # unit + integration tests, and the lit suite (see below)
python test/verify/bell.py   # end-to-end Aer verification against a known reference distribution
```

`cargo test` drives `test/lit/`'s IR-round-trip and emission FileCheck suite too (`quonc/tests/lit.rs`, PRD story 38) — it shells out to `lit`, which needs `lit`/`FileCheck` on `PATH` and the oracle binaries from `cargo build --examples` above; without those it skips with a message instead of failing, so a bare `cargo test` on a fresh checkout stays green. Run `lit test/lit/ -v` directly for verbose per-test output.

`test/verify/*.py` Aer-verifies all 8 PRD reference algorithms (Bell, teleportation, Bernstein-Vazirani, Grover, QFT, Ising, QAOA, Shor) plus SABRE routing, each against a known theoretical output distribution. Those scripts share `python/quon_aer.py` (compile → Qiskit dialect normalize → Aer → oracle); unit tests for the compat rewrite live in `python/test_quon_aer.py`.

CI (`.github/workflows/ci.yml`) runs the same stable checks, the lit suite, and all `test/verify/` scripts on every push and pull request. `flux_verify` is checked separately via `cargo flux` in `.github/workflows/flux.yml`.

### Taskless validation

Project-specific ast-grep rules live in `.taskless/rules/`. Run locally:

```bash
npx @taskless/cli@latest check
```

CI: `.github/workflows/taskless.yml`. See [docs/agents/validation.md](docs/agents/validation.md).

### Flux refinement types

The `flux_verify` crate demonstrates Flux refinement types on a **nightly** toolchain (the rest of the workspace stays on stable).

**Install Flux** (once per machine):

```bash
curl -fsSL https://raw.githubusercontent.com/flux-rs/flux/main/install.sh | bash
```

**Run checks:**

```bash
cargo flux -p flux_verify
```

CI: `.github/workflows/flux.yml` (path-filtered to `flux_verify/`). Requires z3 on PATH.

## Workspace

| Crate | Role |
|---|---|
| `quonc` | Compiler driver binary (`quonc`) |
| `frontend` | Lexer, parser, type checker (linear + Clifford + depth), Z3 refinement, AST→IR lowering |
| `zx` | ZX-graph data structure (`petgraph::StableGraph`) and rewrite engine |
| `mlir_bridge` | Melior wrappers, dialect registration, optimization passes, OpenQASM 3.0 emitter |
| `backend` | `BackendTarget`, noise model, connectivity graph, JSON device loader |
| `quon_core` | MLIR-free shared types (`DepthExpr`, the OpenQASM 3.0 typed IR) used by both `frontend` and `mlir_bridge` without pulling either into the other |
| `quon_na` | Neutral-atom backend: interaction graph, placement, AOD/zoned scheduling, compaction, resource reports |
| `flux_verify` | Flux refinement-type examples (nightly; `cargo flux -p flux_verify`) |

## Documentation

- [SPEC.md](SPEC.md) — full language and compiler specification
- [CONTEXT.md](CONTEXT.md) — domain glossary
- [docs/adr/](docs/adr/) — architectural decision records
- [GitHub Issues](../../issues) — implementation tracker; #1 is the master PRD
