# Quon

An MLIR-based optimizing compiler for quantum programs. Accepts programs in the Quon language — a functional language with linear types and a monadic quantum interface — and emits OpenQASM 3.0 for execution on Qiskit Aer or real hardware.

## Prerequisites

| Dependency | Version | Notes |
|---|---|---|
| Rust | stable | edition 2021+ |
| LLVM + MLIR | 19 | Build with `-DLLVM_ENABLE_PROJECTS=mlir` and C API enabled |
| libz3 | any recent | C API; required at link time |
| Python + Qiskit Aer | 3.10+ | Simulation verification only |

If LLVM 19 is not on your default search path, set `LLVM_SYS_PREFIX=/path/to/llvm19`.

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
cargo test          # frontend unit tests
lit test/lit/       # IR round-trip and emission FileCheck tests
python test/verify/bell.py      # end-to-end Aer verification (Phase 6+)
```

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
