# Fixed physical layout: SSA wiring is the canonical channel; `phys_qubit` is a derived annotation

## Context

`CONTEXT.md` defines `quantum.physical` as `quantum.dynamic` ops annotated with hardware
attributes (`phys_qubit`, `native_gate`, `fidelity`). In the Fixed compile path (issue #316),
layout identity historically forked into two channels:

1. **SSA wiring** — SABRE routing rewrites qubit operands and inserts SWAP gates. OpenQASM
   emit follows this SSA wiring (its `Reifier` threads physical register indices through SSA
   values) and never reads `phys_qubit`.
2. **`phys_qubit` attribute** — SABRE writes a single `I32Attr` per gate (the physical index
   of the first qubit operand) from its `Layout`. Depth scheduling and metrics read it via
   `dynamic_walk::resolve_phys_qubits`, which *folded the attr into the WireTracker root list*.

The fork meant emit and scheduling could disagree on which qubits a gate touches: the attr
fold-in could make scheduling see a spurious extra qubit (root + attr) on a 2-qubit gate,
creating false dependencies. The "deletion test" from the issue confirmed the split: dropping
attr writes broke scheduling while QASM still worked; dropping SSA rewiring broke emit while
attrs lingered.

## Decision

**SSA qubit wiring is the canonical representation of Fixed physical layout identity.**
The `phys_qubit` attribute is a **derived annotation**, not a second source of truth.

- SABRE rewrites SSA operands (canonical) and writes `phys_qubit` attrs from its `Layout`
  (derived, a consequence of SSA rewiring).
- `dynamic_walk::resolve_phys_qubits` derives qubit identity **purely from WireTracker roots**
  (the SSA-identity channel) when roots are available — the attr fold-in is removed. The attr
  fallback remains only for hand-built test fixtures that annotate gates without a tracker.
- OpenQASM emit already follows SSA wiring (unchanged).
- Both scheduling and emit consume the same SSA channel, so they cannot disagree on which
  physical qubit a gate touches.

The Fixed physical pipeline (`native_gate_decomp` → `sabre_routing` → `native_gate_decomp`
→ `depth_scheduling`) is owned by one module: `mlir_bridge::fixed_physical`. Emit, T-count
sampling, and metrics consume that module's result rather than re-deriving layout from a
second channel.

### Why SSA-authoritative (least churn)

Emit already follows SSA wiring and never reads `phys_qubit`. SSA is the natural MLIR
representation of qubit identity — the WireTracker threads it across `unitary_region`/`if`
boundaries. Making SSA authoritative required changing only `resolve_phys_qubits` (remove the
attr fold-in) and documenting the decision. The alternative — making `phys_qubit` authoritative
and updating emit to read it — would require emit to trace attrs through region boundaries (a
new walker) and would force every gate to carry a complete attr set (impossible for multi-qubit
gates, since `phys_qubit` is a single `I32Attr`).

## Consequences

- `resolve_phys_qubits` no longer folds the `phys_qubit` attr into the root list. When roots
  are present (the production pipeline after SABRE), identity is derived purely from them.
  The attr fallback covers only hand-built fixtures without a WireTracker.
- `mlir_bridge::fixed_physical::corrupt_phys_qubit_attrs` is the deletion-test helper: it
  overwrites every `phys_qubit` attr with a bogus value. After routing, emit and scheduling
  are provably unaffected (tests in `tests/fixed_physical.rs`).
- `phys_qubit` remains a valid, optional `I32Attr` on `quantum.dynamic` ops (the verifier is
  unchanged). SABRE continues to write it as a derived annotation; metrics may read it for
  reporting. It is never load-bearing for correctness.
- Neutral-atom scheduling is unchanged: it stays on `quantum.na` / `ScheduleLayer` →
  `ScheduleSpec` (ADR-0007, ADR-0011, ADR-0009) and is NOT part of the `fixed_physical` module.
- QEC paths are unchanged.

## References

- Issue #316 — Compiler: Own Fixed physical layout as one module (SSA vs phys_qubit)
- Issue #82 — (referenced by #316 as the originating layout-identity concern)
- ADR-0009 — Unified `BackendTarget` behind `TargetKind`
- ADR-0007 — `quantum.na` as a separate dialect (neutral-atom scheduling IR)
- ADR-0011 — `quantum.na` is the canonical schedule IR; `ScheduleLayer` is planner-internal
