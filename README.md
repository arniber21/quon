# Quon

Quon is a Rust quantum compiler toolkit for writing typed quantum programs,
checking quantum-specific resource invariant, and lowering them into backend
artifacts: OpenQASM 3 for gate-model workflows, and verified neutral-atom
schedules + QEC experiment artifacts (Stim circuits, resource reports, fused
Stim/Sinter validation reports) for reconfigurable neutral-atom targets.

The project is past the prototype phase and is being shaped into a usable
toolset for quantum software engineers: a typed language frontend, an MLIR
compiler pipeline, a backend target model, a QEC workload IR with Stim/Sinter
evaluation, a verification harness, formatter, linter, LSP, and a packaging
path — in one workspace.

## Toolkit features

- **Typed quantum language.** `Circuit<n, m, d, C>` types, linear `Qubit` /
  `QReg<n>` values, symbolic depth bounds, Clifford classification, and a
  `Q<T>` quantum monad for allocation, measurement, and feed-forward.
- **Static resource checks.** The frontend rejects qubit cloning, unconsumed
  quantum values, invalid register widths, overly tight depth annotations, and
  incorrect Clifford annotations before lowering.
- **Compiler pipeline.** `quonc` parses, desugars, type-checks, elaborates,
  lowers to MLIR generic-form IR, runs circuit and dynamic passes, adapts to
  target constraints, collects metrics, and emits backend artifacts.
- **OpenQASM 3 output.** Fixed gate-model targets compile to typed OpenQASM 3
  through native-gate decomposition, SABRE-style routing, depth scheduling, and
  a validating emitter.
- **Neutral-atom scheduling.** Reconfigurable neutral-atom targets compile to
  schedule JSON and resource reports through interaction-graph extraction,
  entangling-layer scheduling, zoned or flat AOD movement, compaction, and
  timing/resource accounting — with an analytic end-to-end fidelity estimate
  (Enola Eq. (1)).
- **QEC workload IR + Stim/Sinter evaluation.** `quon_qec` owns code-family
  expansion (repetition/surface codes), lattice-surgery logical CX, magic-state
  T/CCZ operations, and dual-emit of a semantic `*.qec.json` + structure-level
  `.stim` circuit. The Python Stim/Sinter harness annotates noise, samples
  logical failures, and produces a fused validation report with provenance.
- **Backend target model.** JSON descriptors model topology, native gates,
  noise/fidelity data, measurement latency, and dynamic-circuit capabilities.
- **Verification seam.** Python tools compile Quon, normalize OpenQASM for
  Qiskit import, run Qiskit Aer, and check reference distributions.
- **Developer toolchain.** `quonfmt`, `quonlint`, `quon_lsp`, Tree-sitter
  grammar, VS Code/Zed/Neovim integration, Devbox bootstrap, CI-parity recipes,
  lit/FileCheck tests, fuzz/property tests, and optional Flux refinement
  examples.

## Quick example: typed frontend to OpenQASM 3

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

## Flagship: neutral-atom fault-tolerant compilation with QEC validation

The strongest end-to-end path: a typed surface-code program compiled into a
**verified neutral-atom schedule**, a **QEC experiment JSON** with a sibling
**Stim circuit**, an **analytic resource report**, and a **fused Stim/Sinter
validation report** — from one compiler invocation.

Compile a distance-3 surface-code logical CX (lattice surgery) and emit the
schedule, resource report, and QEC experiment dual-emit in one command:

```bash
cargo run -p quonc -- examples/na_qec/surface_d3_cx.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule schedule.json \
  --emit-resource-report report.md \
  --emit-qec-experiment /tmp/surface_d3.qec.json
# writes: schedule.json, report.md, /tmp/surface_d3.qec.json + /tmp/surface_d3.stim
# prints: quantum.na verification passed (QEC auto)
```

That single compile produces four artifacts from one `quon_qec` workload IR
pass:

| Artifact | What | Evidence kind |
|---|---|---|
| `schedule.json` | Neutral-atom schedule (zones, layers, metrics) | Compiler |
| `report.md` | Analytic resource report + error budget + fidelity estimate | Analytic |
| `*.qec.json` | QEC experiment IR (family, distance, observables, error_model) | Compiler |
| `*.stim` | Structure-level Stim circuit (detectors, observables) | Evaluation |

For the full QEC evidence story — analytic estimates **fused with**
Stim/Sinter-sampled logical failure rates, tied together by a provenance
fingerprint — run `--emit-qec-validation` (requires the Python/Sinter stack via
`just setup-python`):

```bash
cargo run -p quonc -- examples/na_qec/repetition_d3_memory.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-qec-validation /tmp/rep_d3.validation.json --validation-shots 256
# writes: /tmp/rep_d3.validation.json + .md (analytic + sampled sections, provenance)
```

See the [neutral-atom FT compiler demo](https://quon.arnabg.me/guides/na-ft-demo/)
for the full walked-through example with excerpts of every artifact.

## Install and build

### Recommended setup: Devbox

Devbox pins LLVM/MLIR 22, Z3, Python, Node, `just`, `sccache`, and
`cargo-nextest`. Rust itself is selected by `rust-toolchain.toml`.

```bash
git clone https://github.com/arniber21/quon.git
cd quon
devbox shell          # or: direnv allow
just doctor           # check required tools
cargo build -p quonc  # build the compiler
```

`devbox.json` sets `MLIR_SYS_220_PREFIX` from `llvm-config --prefix` via the
local `nix/llvm-mlir` flake, so Melior sees a single LLVM/MLIR prefix.
`sccache` is wired as `RUSTC_WRAPPER` automatically — incremental rebuilds
after `cargo clean` or across branches hit the cache.

### Running the full test suite

Everything goes through `just` (inside `devbox shell` or via
`devbox run -- just …`):

```bash
just doctor           # toolchain readiness matrix
just test-fast        # unit + integration tests (cargo-nextest; no Aer/lit)
just test-ci          # FULL local CI-parity path (run before pushing)
```

`just test-ci` runs the same gates as CI:

- `cargo fmt --check` + `cargo clippy -- -D warnings`
- release build + examples build
- `cargo nextest run --workspace` (Rust tests, including the sample catalog lint)
- lit/FileCheck tests (`QUON_REQUIRE_LIT=1`)
- Python Aer verification seam + Stim/Sinter smoke + QEC benchmarks
- `quonfmt` / `quonlint` / `quon_lsp` tooling smoke
- validation-doc assertions

The Python harness (Aer bridge, Stim/Sinter, QEC benchmarks, NA visualization)
needs a one-time setup:

```bash
just setup-python     # creates .venv and installs python/requirements.txt
.venv/bin/python -m pytest python -q   # run all Python tests
```

### Manual source setup

Prefer Devbox. If you manage the native dependencies yourself, install
LLVM/MLIR 22 and libz3, then set `MLIR_SYS_220_PREFIX` to the LLVM
installation root.

**macOS:**

```bash
brew install llvm@22 z3 python
export MLIR_SYS_220_PREFIX="$(brew --prefix llvm@22)"
export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
```

**Ubuntu/Debian:** use [apt.llvm.org](https://apt.llvm.org/) for LLVM 22, plus
`libz3-dev`, `libmlir-22-dev`, `llvm-22-dev`.

### Release packaging path

Prebuilt CLIs (`quonc`, `quonfmt`, `quon_lsp`, `quonlint`) that need no local
LLVM/MLIR/Z3 install:

- `scripts/install.sh` — curl-style installs from GitHub Releases.
- `scripts/package-deb.sh` — Debian packages.
- `scripts/generate-homebrew-formula.sh` — Homebrew formula generation.
- `scripts/release.sh` — static release assembly.
- `.github/workflows/release.yml` — tagged release builds (linux + macOS).

## CLI usage

```bash
# Compile to OpenQASM 3 with the default all-to-all fixed target.
cargo run -p quonc -- test/verify/bell.qn --emit-qasm

# Compile for a fixed target descriptor with metrics.
cargo run -p quonc -- test/verify/bernstein_vazirani.qn \
  --target backend/tests/fixtures/device_5q.json \
  --emit-qasm --metrics

# Neutral-atom schedule + interaction graph + resource report.
cargo run -p quonc -- test/na/qaoa_graph.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule schedule.json \
  --emit-na-graph graph.dot \
  --emit-resource-report

# QEC experiment dual-emit (JSON + Stim) + fused validation report.
cargo run -p quonc -- examples/na_qec/repetition_d3_memory.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-qec-validation /tmp/rep_d3.validation.json

# Inspect the compiler pipeline or dump intermediate IR.
cargo run -p quonc -- --list-passes
cargo run -p quonc -- test/verify/bell.qn --dump-ir --emit-qasm
```

See the [quonc CLI reference](https://quon.arnabg.me/reference/quonc/) for
every flag.

## Compiler pipeline

```text
Quon source
  -> parse / desugar / typecheck
  -> elaborate parametric circuits
  -> lower to quantum.circ + quantum.dynamic (run blocks lower straight
     to dynamic IR; no staging dialect — #213 / ADR-0037)
  -> circuit simplification passes
  -> dynamic passes
  -> fixed OpenQASM path OR neutral-atom schedule path
```

The fixed path runs native-gate decomposition, routing, post-routing
decomposition, scheduling, metrics, and OpenQASM 3 emission. The neutral-atom
path extracts an interaction graph, schedules entangling layers, applies zoned
or flat movement planning, compacts the schedule, and emits schedule/resource
artifacts — with QEC experiment dual-emit and Stim/Sinter validation for
QEC-backed programs.

The custom IRs are MLIR generic-form operations through Melior. Quon uses
explicit Rust builders and verifiers around unregistered dialects rather than a
C++/TableGen dialect build.

## Verification and testing

```bash
just doctor      # toolchain readiness
just test-fast   # fast Rust-focused path (cargo-nextest)
just test-ci     # full local CI-parity path
```

Reference verification scripts in `test/verify/` cover Bell, teleportation,
Bernstein-Vazirani, Grover, QFT, Ising, QAOA, Shor's quantum kernel, and
routing on constrained fixed targets. Narrative demos beyond those CI fixtures
live in `samples/`, indexed by `samples/catalog.yaml` (ADR-0025); see
`samples/README.md`.

## Workspace

| Crate | Role |
|---|---|
| `quonc` | Compiler driver binary |
| `frontend` | Lexer, parser, desugaring, typechecker, refinement checks, AST lowering |
| `mlir_bridge` | Melior IR builders, dialect wrappers, passes, metrics, OpenQASM emitter |
| `backend` | Target descriptors, topology, native gates, noise model, JSON loader |
| `quon_core` | MLIR-free shared kernels and typed OpenQASM data model |
| `quon_na` | Neutral-atom interaction graph, placement, movement, scheduling, resource reports |
| `quon_qec` | QEC workload IR: code families, expansion, Stim/experiment dual-emit, lattice surgery, magic-state ops |
| `zx` | ZX graph representation and rewrite engine |
| `quonfmt` | Formatter |
| `quonlint` / `quonlint-cli` | Algorithm-quality linter |
| `quon_lsp` | Language server |
| `flux_verify` | Optional Flux refinement examples |

## Documentation

- [SPEC.md](SPEC.md) — language and compiler specification.
- [CONTEXT.md](CONTEXT.md) — domain glossary and project vocabulary.
- [CHANGELOG.md](CHANGELOG.md) — release history.
- [CONTRIBUTING.md](CONTRIBUTING.md) — contributor setup and workflow.
- [docs/adr/](docs/adr/) — architectural decisions.
- [samples/README.md](samples/README.md) — sample corpus taxonomy, catalog, and contribution guide.
- [docs/neutral_atom/architecture_model.md](docs/neutral_atom/architecture_model.md) — neutral-atom model and citations.
- [website/](website/) — Starlight docs site at https://quon.arnabg.me.

## Maturation path

Quon is growing along a deliberate production-tooling path:

- make installation increasingly boring through self-contained releases;
- keep the compiler pipeline reproducible with CI-parity local commands;
- make backend artifacts more inspectable with schedule/IR visualization;
- deepen neutral-atom and QEC validation with benchmark regressions and
  Stim/Sinter evidence;
- broaden optimization coverage while preserving verifier-backed correctness;
- connect more static invariants to backend legality and resource accounting.

Roadmap issues are tracked openly, but the repository's public surface is built
around the toolkit capabilities already represented in source, tests, docs, and
command-line workflows.

## License

Quon is licensed under the [Business Source License 1.1](LICENSE) (BSL), which
converts to Apache License 2.0 after the Change Date.
