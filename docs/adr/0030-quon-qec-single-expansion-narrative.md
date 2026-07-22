# Single QEC expansion narrative — retire `LogicalOp` / `expand_code_block`

Date: 2026-07-21 · Issue: #319 · Supersedes part of: [ADR-0015](0015-quon-qec-shared-workload-crate.md)

## Context

ADR-0015 moved the MLIR-free QEC workload IR into a shared `quon_qec` crate and
required the sizing helpers then living in `quon_na::qec` to migrate or re-export
through it. That migration shipped the **production** path:

```
QecWorkload → quon_qec::expand_workload → ExpandedWorkload
            → hybrid schedule / Stim / experiment dual-emit
```

A **parallel era** was left behind in `quon_na::qec`: the `LogicalOp` enum and
`expand_code_block`, a toy "code block expander" that built a `CodeBlock` of
consecutive `AtomId`s from a `CodeFamily` + logical qubits. `LogicalOp` was
essentially unused by hybrid scheduling — it appeared only in a toy example and
its own serialization test. `expand_code_block` was used only by report
fixtures/snapshots and the example; production (`run_from_qec_workload`)
already built `CodeBlock`s from `ExpandedWorkload` via a *private*
`code_blocks_from_expanded` helper.

The **deletion test** made the asymmetry plain: deleting `LogicalOp` broke
almost nothing in production; deleting `expand_workload` broke Stim, hybrid, and
experiment emit. The legacy surface was pass-through — it failed the deletion
test. Understanding "code block expansion" required bouncing between crates and
eras.

## Decision

There is **one expansion narrative**: `ExpandedWorkload` (and `CodeBlock`
derived from it) is the only mental model.

1. **Delete `LogicalOp`.** It had no production consumer and no Quon
   source-language representation.
2. **Delete `expand_code_block`** (the toy expander). Its role of building
   report-shaped `CodeBlock`s is now served by a single canonical helper.
3. **Promote `code_blocks_from_expanded` to the public `quon_na::qec` API** —
   moved out of `qec_schedule.rs` (where it was private) so the hybrid schedule
   path and report fixtures share one derivation from `ExpandedWorkload`. One
   `ExpandedBlock` (one logical qubit in the expansion IR) → one `CodeBlock`,
   mapping `PhysicalAtomId` → `AtomId`.
4. **Family sizing formulas stay only in `quon_qec`** (`atoms_per_logical`,
   `repetition_n`, `surface_n`, `ceil_div`), re-exported from `quon_na::qec`.
5. Report fixtures and snapshots that previously called `expand_code_block` now
   build `CodeBlock`s from the production IR
   (`WorkloadBuilder → expand_workload → code_blocks_from_expanded`) for
   distance-bearing families (`Repetition` / `Surface`). For **sizing-only**
   families with no physical round expansion (`HighRateQldpcLike` /
   `AbstractBlockCode`), fixtures size a `CodeBlock` directly from the
   `quon_qec` formula (`atoms_per_logical`) — these families have no
   `ExpandedWorkload` and never had one; this is report sizing, not expansion.
6. The `repetition_code_toy` example was rewritten to demonstrate the
   production path end-to-end and no longer references `LogicalOp`.

## Consequences

- `quon_na`'s public surface no longer exposes `LogicalOp` or
  `expand_code_block`; `code_blocks_from_expanded` replaces the latter.
- Stim / experiment / hybrid-schedule observable artifacts are unchanged
  (production already used this path; the private helper simply moved and became
  public). Snapshot goldens stay green — the migration is behavior-preserving.
- A `compile_fail` doc-test in `quon_na::qec` asserts the legacy surface is
  gone; `qec` unit tests pin that `code_blocks_from_expanded` reproduces the
  retired expander's atom layout (regression net).
- `mlir_bridge::collect_qec_workload` is intentionally untouched (ADR-0015
  scopes its placement; moving it would require a new ADR).
- Future families needing physical-round expansion add a `SourceFamily` variant
  and an `expand_*_layout`; sizing-only families remain formula-driven.
