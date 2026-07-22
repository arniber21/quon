# SpecializedCircuit module

Status: Accepted · 2026-07-22 · Refs #206, #201, #209, #213, ADR-0028

## Context

Parametric specialization (`frontend/src/elaborate.rs`, issue #1) already produced a
first-order gate tree — `Compose` / `GateApp` / `Adjoint` over concrete qubit indices and
literal rotation angles — but the interface between specialization and lowering stayed
surface `Expr`, with no place for the resolved `Circuit<n,m,d,C>` widths to live. Two
consequences followed:

- `lower.rs` (the `quantum.circ` MLIR adapter) re-derived the widths ad hoc at every call
  site — `specialize_named_fn` re-evaluated the classical arguments, re-elaborated the
  body, and re-substituted the `Circuit` indices inline; `resolve_circuit_callee` sized an
  anonymous circuit from `max_qubit_index`/`count_gates` it also kept locally. The
  specialization *pass* and the MLIR *emission* were one tangled method.
- The helpers `inverse_gate_name`, `collect_gate_placements`, and `flatten_app` were
  copy-pasted across `elaborate.rs` and `lower.rs`; `max_qubit_index` / `count_gates` /
  `qubit_targets` / `literal_usize` lived only in `lower.rs` despite being Melior-free.

Because specialization had no Melior-free home, tests for it could only run linked against
LLVM/MLIR — the opposite of the "MLIR-free shared kernel" boundary the workspace
maintains for everything else cross-cutting the frontend↔IR seam.

## Decision

Introduce a deep **SpecializedCircuit** module —
`frontend/src/specialized_circuit.rs` — as the Melior-free interface between `elaborate`
and `lower`, with three responsibilities (mirroring the issue's stated shape):

- **Interface.** `SpecializedCircuit { body, in_qubits, out_qubits, depth, clifford }` —
  the elaborator's output and lower's only input: the first-order gate DAG (an
  `Sp<Expr>` `Compose`/`GateApp`/`Adjoint` tree) plus the resolved `Circuit<n,m,d,C>`
  indices. No classical parameters remain. Constructed by `SpecializedCircuit::specialize`
  (a parametric call site: evaluate args, elaborate the body, read widths off
  `ParametricDef::ret_ty`) or `::anonymous` (a compound circuit expression sized from its
  highest qubit index and gate count).
- **Implementation.** Specialization, adjoint/inverse normalization, and placement live
  here, Melior-free: `SpecializedCircuit::specialize` / `::cache_key` / `::adjoint`,
  plus the single home for `collect_gate_placements`, `inverse_gate_name`,
  `reverse_and_invert`, `flatten_app`, `max_qubit_index`, `count_gates`, `qubit_targets`.
- **Adapter.** `lower.rs` keeps only the Melior emission: `emit_specialized` lowers a
  `SpecializedCircuit` to a `quantum.circ.func`; `specialize_named_fn` and
  `resolve_circuit_callee` build a `SpecializedCircuit` and hand it to the adapter.

### Type shape — reuse the elaborate `Expr` tree

`SpecializedCircuit` wraps the existing `Compose`/`GateApp`/`Adjoint` `Expr` tree rather
than introducing a parallel IR. Rationale: `elaborate::elaborate_circuit_body` already
produces this tree and is heavily recursive; a new enum would double the AST-walking
surface for no semantic gain. The typed wrapper adds the metadata lower needs (widths,
depth, Clifford) and the helpers that operated on the bare tree, without a second
representation. `lower` flattens the tree via `collect_gate_placements` exactly as before.

### Crate home — `frontend`

`specialized_circuit.rs` lives in `frontend`, next to `elaborate` and `lower`. It depends
only on `quon_core` (`DepthExpr`, the gate registry), `crate::ast`, `crate::elaborate`,
`crate::typecheck::circuit`, and `crate::types` — all Melior-free. It does **not** touch
`melior` or `mlir_bridge`. Keeping it in `frontend` (not `quon_core`) avoids coupling the
shared kernel to frontend concepts (the `Expr`/`Ty` types); keeping it out of `mlir_bridge`
keeps the MLIR adapter a thin leaf.

### Feature surface — `analyze` vs `full`

Split the frontend features so specialization is buildable and testable without LLVM/MLIR:

- `analyze` = `dep:quon_core` + `dep:z3` (no `mlir_bridge`/`melior`) gates the Melior-free
  pipeline: `analysis`, `desugar`, `elaborate`, `refinement`, `typecheck`, `types`,
  `specialized_circuit`, and the `diagnostics` items those need (`AnalysisResult`,
  `fixes`).
- `full` = `analyze` + `dep:mlir_bridge` + `dep:melior` gates only `lower` (the
  `quantum.circ` adapter) and `lower_program_to_mlir`.

`cargo build -p frontend --no-default-features --features analyze` compiles specialization
with no Melior link; `quonfmt`'s `default-features = false` (parser only) is unchanged;
`quonc` / `quon_lsp` (default `full`) get the whole frontend.

## Consequences

- Specialization, adjoint/inverse normalization, and placement are testable in isolation
  — `specialized_circuit::tests` (unit + proptest) exercise them with no `melior` /
  `mlir_bridge` import. The module is verifiably Melior-free at the build-graph level.
- `inverse_gate_name`, `collect_gate_placements`, `flatten_app`, and the width/depth
  readers live once. `elaborate` and `lower` import them from `specialized_circuit`; their
  local copies are deleted.
- `lower::specialize_named_fn` keeps its memoization (reserve-before-recurse against the
  structurally-impossible self-call) but delegates the pure specialization to
  `SpecializedCircuit::specialize`; `cache_key` short-circuits on a hit before elaborating.
- Behavior is preserved: `reverse_and_invert` (now shared) rebuilds the same right-leaning
  `Compose` tree the elaborate and lower copies did; `lower` flattens it via
  `collect_gate_placements` as before, so emit / lit / Aer output is unchanged.
- The `SpecializedCircuit` term is recorded in `CONTEXT.md` (distinct from `CircIr` — the
  `quon_circ` flat IR consumed by unitary optimization).
