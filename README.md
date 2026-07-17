# Quon

Quon is a Rust quantum compiler toolkit for writing typed quantum programs,
checking quantum-specific resource invariants, and lowering programs into
backend artifacts: OpenQASM 3 for gate-model workflows and schedule/resource
reports for reconfigurable neutral-atom targets.

The project is past the prototype phase and is being shaped into a usable
toolset for quantum software engineers: a language frontend, compiler pipeline,
backend target model, verification harness, formatter, linter, LSP, and
packaging path in one workspace.

## Toolkit Features

- **Typed quantum language.** Quon programs use `Circuit<n, m, d, C>` types,
  linear `Qubit` / `QReg<n>` values, symbolic depth bounds, Clifford
  classification, and a `Q<T>` quantum monad for allocation, measurement, and
  feed-forward.
- **Static resource checks.** The frontend rejects qubit cloning, unconsumed
  quantum values, invalid register widths, overly tight depth annotations, and
  incorrect Clifford annotations before lowering.
- **Compiler pipeline.** `quonc` parses, desugars, type-checks, elaborates,
  lowers to MLIR generic-form IR, runs circuit and dynamic passes, adapts to
  target constraints, collects metrics, and emits backend artifacts.
- **OpenQASM 3 output.** Fixed gate-model targets compile to typed OpenQASM 3
  through native-gate decomposition, SABRE-style routing, depth scheduling, and
  a validating emitter.
- **Backend target model.** JSON descriptors model topology, native gates,
  noise/fidelity data, measurement latency, and dynamic-circuit capabilities.
- **Neutral-atom scheduling.** Reconfigurable neutral-atom targets compile to
  schedule JSON and resource reports through interaction-graph extraction,
  entangling-layer scheduling, zoned or flat AOD movement, compaction, and
  timing/resource accounting.
- **Verification seam.** Python tools compile Quon, normalize OpenQASM for
  Qiskit import, run Qiskit Aer, and check reference distributions.
- **Developer toolchain.** The workspace includes `quonfmt`, `quonlint`,
  `quon_lsp`, Tree-sitter grammar, VS Code/Zed/Neovim integration, Devbox
  bootstrap, CI-parity recipes, lit/FileCheck tests, fuzz/property tests, and
  optional Flux refinement examples.

## Quick Example

```qn
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}

fn main(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0       <- measure(q0)
    b1       <- measure(q1)
    return (b0, b1)
}
```

Compile it to OpenQASM 3:

```bash
cargo run -p quonc -- test/verify/bell.qn --emit-qasm
```

Expected shape:

```qasm
OPENQASM 3.0;
include "stdgates.inc";
qubit[2] q;
bit[2] c;
h q[0];
cx q[0], q[1];
c[0] = measure q[0];
c[1] = measure q[1];
```

Compile the same style of program for the neutral-atom schedule path:

```bash
cargo run -p quonc -- test/na/bell.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule \
  --emit-resource-report
```

For the strongest end-to-end path — a typed surface-code program compiled into a
verified neutral-atom schedule, QEC experiment JSON, structure Stim, and an
analytic resource report from one command — see the
[neutral-atom FT compiler demo](https://quon.arnabg.me/guides/na-ft-demo/).

## Install and Build

### Recommended contributor setup

Use Devbox for LLVM/MLIR 22, Z3, Python, Node, and `just`:

```bash
git clone https://github.com/arniber21/quon.git
cd quon
devbox shell
just doctor
cargo build -p quonc
```

`devbox.json` pins the native toolchain and sets `MLIR_SYS_220_PREFIX` for
Melior. Rust itself is selected by `rust-toolchain.toml`.

### Manual source setup

Install LLVM/MLIR 22 and libz3, then set:

```bash
export MLIR_SYS_220_PREFIX=/path/to/llvm22
export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
```

On macOS with Homebrew:

```bash
brew install llvm@22 z3
export MLIR_SYS_220_PREFIX="$(brew --prefix llvm@22)"
export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
```

On Ubuntu/Debian, use [apt.llvm.org](https://apt.llvm.org/) for LLVM 22 and
install `libz3-dev`.

### Release packaging path

The project includes scripts for self-contained release artifacts:

- `scripts/install.sh` for curl-style installs from GitHub Releases.
- `scripts/package-deb.sh` for Debian packages.
- `scripts/generate-homebrew-formula.sh` for Homebrew formula generation.
- `scripts/release.sh` for static release assembly.

These scripts make the intended release path concrete: prebuilt CLIs that do
not require a local LLVM/MLIR or Z3 install. Contributors building from source
should still use Devbox.

## CLI Usage

```bash
# Compile to OpenQASM 3 with the default all-to-all fixed target.
cargo run -p quonc -- test/verify/bell.qn --emit-qasm

# Compile for a fixed target descriptor.
cargo run -p quonc -- test/verify/bernstein_vazirani.qn \
  --target backend/tests/fixtures/device_5q.json \
  --emit-qasm \
  --metrics

# Emit a neutral-atom schedule view, interaction-graph DOT, and resource report.
cargo run -p quonc -- test/na/qaoa_graph.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule schedule.json \
  --emit-na-graph graph.dot \
  --emit-resource-report

# Render schedule frames (matplotlib) and/or the graph (Graphviz).
pip install -r python/requirements-viz.txt
python python/visualize_na_schedule.py schedule.json --graph graph.dot \
  -o /tmp/na-viz --format svg

# Inspect the compiler pipeline.
cargo run -p quonc -- --list-passes

# Dump intermediate IR checkpoints.
cargo run -p quonc -- test/verify/bell.qn --dump-ir --emit-qasm
```

## Compiler Pipeline

`quonc` runs a shared frontend and optimization path before selecting a backend
artifact:

```text
Quon source
  -> parse / desugar / typecheck
  -> elaborate parametric circuits
  -> lower to quantum.circ
  -> circuit simplification passes
  -> monadic lowering to quantum.dynamic
  -> dynamic passes
  -> fixed OpenQASM path OR neutral-atom schedule path
```

The fixed path runs native-gate decomposition, routing, post-routing
decomposition, scheduling, metrics, and OpenQASM 3 emission. The neutral-atom
path extracts an interaction graph, schedules entangling layers, applies zoned
or flat movement planning, compacts the schedule, and emits schedule/resource
artifacts.

The custom IRs are represented as MLIR generic-form operations through Melior.
Quon uses explicit Rust builders and verifiers around unregistered dialects
rather than a C++/TableGen dialect build.

## Verification and Testing

```bash
just doctor
just test-fast
just test-ci
```

`just test-ci` is the local CI-parity path: formatting, clippy, release build,
example binaries for lit/FileCheck, Rust tests, Python Aer verifiers, tooling
checks, validation-doc assertions, and the sample corpus catalog lint.

The Python harness tests (Aer bridge, Stim/Sinter, QEC benchmarks, NA
visualization) can also be run in one shot after `just setup-python`:

```bash
.venv/bin/python -m pytest python -q
```

The QEC-benchmark integration tests invoke `quonc`, preferring
`target/release/quonc` — build it first (`cargo build --release -p quonc`) or
point `QUONC` at a debug binary.

Reference verification scripts live in `test/verify/` and cover Bell,
teleportation, Bernstein-Vazirani, Grover, QFT, Ising, QAOA, Shor's quantum
kernel, and routing on constrained fixed targets. Narrative demos beyond
those CI fixtures live in `samples/`, indexed by `samples/catalog.yaml`
(ADR-0025); see `samples/README.md`.

## Workspace

| Crate | Role |
|---|---|
| `quonc` | Compiler driver binary |
| `frontend` | Lexer, parser, desugaring, typechecker, refinement checks, AST lowering |
| `mlir_bridge` | Melior IR builders, dialect wrappers, passes, metrics, OpenQASM emitter |
| `backend` | Target descriptors, topology, native gates, noise model, JSON loader |
| `quon_core` | MLIR-free shared kernels and typed OpenQASM data model |
| `quon_na` | Neutral-atom interaction graph, placement, movement, scheduling, resource reports |
| `zx` | ZX graph representation and rewrite engine |
| `quonfmt` | Formatter |
| `quonlint` / `quonlint-cli` | Algorithm-quality linter |
| `quon_lsp` | Language server |
| `flux_verify` | Optional Flux refinement examples |

## Documentation

- [SPEC.md](SPEC.md) - language and compiler specification.
- [CONTEXT.md](CONTEXT.md) - domain glossary and project vocabulary.
- [docs/adr/](docs/adr/) - architectural decisions.
- [samples/README.md](samples/README.md) - sample corpus taxonomy, catalog, and contribution guide.
- [docs/neutral_atom/architecture_model.md](docs/neutral_atom/architecture_model.md) - neutral-atom model and citations.
- [docs/plans/m5-closeout-audit.md](docs/plans/m5-closeout-audit.md) - M5 close-out audit and implementation evidence.
- [website/](website/) - Starlight docs site.

## Maturation Path

Quon is growing along a deliberate production-tooling path:

- make installation increasingly boring through self-contained releases;
- keep the compiler pipeline reproducible with CI-parity local commands;
- make backend artifacts more inspectable with schedule/IR visualization;
- deepen neutral-atom validation with benchmark regressions and first-class
  schedule IR;
- broaden optimization coverage while preserving verifier-backed correctness;
- connect more static invariants to backend legality and resource accounting.

Roadmap issues are tracked openly, but the repository's public surface is built
around the toolkit capabilities already represented in source, tests, docs, and
command-line workflows.
