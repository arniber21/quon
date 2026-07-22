# Shared circ extract/rebuild seam for ZX + Clifford+T (#320)

ZX simplification (#75) and Clifford+T optimization (#96 / ADR-0013) both need
to extract a `quantum.circ.func` into a Melior-free gate+wire IR, run their
kernel, and rebuild verified `quantum.circ` ops. Before this ADR, the ZX pass
owned a private, lossy extract/rebuild in `mlir_bridge::passes::zx_simplification`
that keyed qubits by **operand position** (not SSA wire identity), silently
dropped unsupported gate names, and only handled single-qubit funcs.
`clifford_t_opt` (#96) was a reserved alias with no kernel.

## Decision

**ONE shared extract/rebuild module** — `mlir_bridge::circ_extract` — sits
between Melior and the optimization kernels. ZX and Clifford+T call it; they do
not own Melior walking.

### Extract (`circ_extract::extract`)

Walks a `quantum.circ.func` body in order, tracking logical wire indices through
SSA values via `WireTracker` (`mlir_bridge::passes::qubit_wiring`). Each
`quantum.circ.gate` op becomes a `CircGate` whose:

- `name` is the **canonical registry id** from `quon_core::gates` (issue #209),
  not the raw MLIR attribute string — `CX` → `CNOT`, `S†` → `S_dag`.
- `qubits` are faithful logical wire indices recovered from SSA, not operand
  positions. After `CNOT(0,1)`, a gate on wire `1` records `[1]`, not `[0]`.
- `angle`, `depth_contribution`, `clifford` are preserved from the op.

### Rebuild (`circ_extract::rebuild`)

Threads fresh wires from block arguments, applies each `CircGate` via the
verified `quantum.circ` builders, and repoints the terminator in place. The
original ops are erased only after the new ones are wired, so the IR is never
left with dangling uses.

### Faithfulness contract — fail CLOSED

`extract` returns `Err(SeamError)` when the round-trip cannot be faithful:

- circuit-level ops (`compose`, `tensor`, `adjoint`, `controlled`, `borrow`) —
  these cannot be flattened into a gate sequence without losing semantics,
- gate names not in the registry,
- operand count ≠ registry arity.

Consumers treat `Err` as **decline rewrite, leave the func unchanged**. No gate
is ever silently dropped. This is the key correctness improvement over the old
ZX adapter, which silently dropped unsupported names via a `_ => {}` match arm.

### ZX kernel interop

`circ_gate_to_gate_ref` / `gate_ref_to_circ_gate` convert between `CircGate`
and the `zx` crate's `GateRef`, bridging the seam to the ZX graph kernel. The
ZX pass uses these to feed `CircIr` → `circuit_to_zx` → `simplify` →
`zx_to_circuit` → `CircIr` → `rebuild`.

## Considered Options

- **Keep the private ZX extract/rebuild, add a second one for Clifford+T** —
  rejected. Two lossy adapters would drift; the operand-position bug would be
  duplicated.
- **Put CircIr types in a new `quon_circ` crate** — deferred. CONTEXT.md
  anticipated a `quon_circ` crate, but the types are small and the
  extract/rebuild functions need Melior. Living in `mlir_bridge::circ_extract`
  is the minimal correct home until a standalone crate is warranted by additional
  Melior-free consumers.
- **Support borrow/adjoint/controlled in the flat IR** — rejected. These are
  structural constructs that cannot be flattened without losing semantics.
  The seam declines them; kernels that need structure (e.g. Clifford+T
  gadgetization) will extend the IR when implemented (#96).
- **Canonicalize gate names during rebuild instead of extract** — rejected.
  Canonicalizing at extract means the CircIr is always in canonical form, so
  kernels never see aliases and the `zx_encodable` check is simplified.

## Consequences

- The ZX pass (`zx_simplification.rs`) no longer owns Melior walking; it calls
  `circ_extract::extract` / `rebuild`. The single-qubit restriction is preserved
  (the ZX kernel still can't handle multi-qubit); multi-qubit ZX is #75.
- `clifford_t_opt` (#96) will call the same seam when its kernel lands.
- The old `extract_gates` is kept as a thin public wrapper for test
  compatibility; it delegates to `circ_extract::extract`.
- Gate names in the CircIr are always canonical, so no parallel string tables.
- CONTEXT.md `CircIr` entry updated to reflect `mlir_bridge::circ_extract` as
  the seam's home.

## References

- #320 — this issue
- #75 — ZX simplification (multi-qubit extraction deferred)
- #96 — Clifford+T optimization (ADR-0013)
- #209 — gate registry (`quon_core::gates`)
- ADR-0013 — Clifford+T RM + tableau decision
