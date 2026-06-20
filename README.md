# Quon

An MLIR-based optimizing compiler for quantum programs. Accepts programs in the Quon language — a functional language with linear types and a monadic quantum interface — and emits OpenQASM 3.0 for execution on Qiskit Aer or real hardware.

## Prerequisites

| Dependency | Version | Notes |
|---|---|---|
| Rust | stable | edition 2021+; see `rust-toolchain.toml` |
| LLVM + MLIR | **22** | Build with `-DLLVM_ENABLE_PROJECTS=mlir` and the C API enabled |
| Melior | **0.27.x** | Pinned in the workspace `Cargo.toml`; requires LLVM 22 |
| libz3 | any recent | C API; required at link time by the `z3` crate (`frontend`) |
| Python + Qiskit Aer | 3.10+ | Simulation verification only (Phase 6+) |

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
```

## Testing

```bash
cargo fmt --all -- --check   # formatting (also enforced in CI)
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace       # unit + integration tests across all crates
lit test/lit/                # IR round-trip and emission FileCheck tests (later phases)
python test/verify/bell.py   # end-to-end Aer verification (Phase 6+)
```

CI (`.github/workflows/ci.yml`) runs `fmt`, `clippy`, `cargo build --release`, and `cargo test --workspace` on every push and pull request.

## Workspace

| Crate | Role |
|---|---|
| `quonc` | Compiler driver binary (`quonc`) |
| `frontend` | Lexer, parser, type checker (linear + Clifford + depth), Z3 refinement, AST→IR lowering |
| `zx` | ZX-graph data structure (`petgraph::StableGraph`) and rewrite engine |
| `mlir_bridge` | Melior wrappers, dialect registration, optimization passes, OpenQASM 3.0 emitter |
| `backend` | `BackendTarget`, noise model, connectivity graph, JSON device loader |

## Documentation

- [SPEC.md](SPEC.md) — full language and compiler specification
- [CONTEXT.md](CONTEXT.md) — domain glossary
- [docs/adr/](docs/adr/) — architectural decision records
- [GitHub Issues](../../issues) — implementation tracker (issues #2–#30)
