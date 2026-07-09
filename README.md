# Quon

An MLIR-based optimizing compiler for quantum programs. Accepts programs in the Quon language — a functional language with linear types and a monadic quantum interface — and emits OpenQASM 3.0 for execution on Qiskit Aer or real hardware.

## Prerequisites

| Dependency | Version | Notes |
|---|---|---|
| Rust | stable | edition 2024; see `rust-toolchain.toml` |
| LLVM + MLIR | **22** | Build with `-DLLVM_ENABLE_PROJECTS=mlir` and the C API enabled |
| Melior | **0.27.x** | Pinned in the workspace `Cargo.toml`; requires LLVM 22 |
| libz3 | any recent | C API; required at link time by the `z3` crate (`frontend`) |
| Python + Qiskit Aer | 3.10+ | Simulation verification (`test/verify/`) |
| Flux (optional) | nightly + z3 | Refinement types in `flux_verify`; install via [Flux install script](https://flux-rs.github.io/flux/guide/install.html) |

If LLVM 22 is not on your default search path, set:

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

# Simulate with Qiskit Aer
./target/release/quonc program.qn --emit-qasm | python python/quon_aer.py --shots 4096

# Experiment loop: live metrics and watch mode (see docs/agents/experiment-loop.md)
./target/release/quonc program.qn --watch --target device.json --metrics
```

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

`test/verify/*.py` Aer-verifies all 8 PRD reference algorithms (Bell, teleportation, Bernstein-Vazirani, Grover, QFT, Ising, QAOA, Shor) plus SABRE routing, each against a known theoretical output distribution.

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
| `flux_verify` | Flux refinement-type examples (nightly; `cargo flux -p flux_verify`) |

## Documentation

- [SPEC.md](SPEC.md) — full language and compiler specification
- [CONTEXT.md](CONTEXT.md) — domain glossary
- [docs/adr/](docs/adr/) — architectural decision records
- [GitHub Issues](../../issues) — implementation tracker; #1 is the master PRD
