---
title: Compiler pipeline
description: High-level reference for the pipeline run by quonc.
---

`quonc` runs a shared frontend and MLIR-backed lowering path before selecting a
target-specific artifact: OpenQASM 3 for fixed gate-model targets, or
schedule/resource outputs for reconfigurable neutral-atom targets.

## Pipeline overview

1. **Parse and desugar.** The frontend parses `.qn` source and desugars surface
   syntax into the core AST.
2. **Typecheck and elaborate.** The linear typechecker validates declarations,
   quantum ownership, depth bounds, register widths, and Clifford
   classifications. Parametric circuit calls are specialized at concrete call
   sites via the Melior-free **SpecializedCircuit** module — the typed boundary
   between elaboration and lowering (ADR-0038, #206). Specialization runs
   without linking LLVM/Melior, enabling fast unit and property testing.
3. **Lower to MLIR generic-form IR.** Circuit functions lower through explicit
   Rust builders around unregistered `quantum.circ` operations. `run` blocks
   lower directly into `quantum.dynamic` operations — the ephemeral
   `monadic_staging` dialect was collapsed into `quantum.dynamic` so only two
   quantum dialects exist in the IR: `quantum.circ` and `quantum.dynamic`
   (ADR-0037, #213).
4. **Optimize circuit IR.** Circuit-level passes simplify gates and rotations,
   run cancellation/merging, perform compiler uncomputation, apply bounded
   ZX simplification where the representation supports it, and run **real
   Clifford+T optimization** (ADR-0039, #96):
   - **Phase polynomial pass** (for `clifford=false` circuits): extracts
     non-Clifford content as linear Boolean phase terms, merges/cancels
     algebraically across non-adjacent gates, and re-synthesizes — reducing
     T-count beyond peephole adjacency.
   - **Aaronson-Gottesman stabilizer tableau** (for `clifford=true` circuits):
     simulates Clifford gates via a CHP tableau over GF(2), detects identity
     and single-Pauli sequences across non-adjacent patterns, and replaces
     them.
5. **Lower dynamic behavior.** Measurement deferral and classical-region
   fusion normalize dynamic behavior. `run` blocks are already in
   `quantum.dynamic` from step 3 — no separate staging pass.
6. **Select the backend path.** Fixed targets run the consolidated
   **Fixed physical layout module** (ADR-0034, #316): native-gate decomposition,
   SABRE-style routing, post-routing decomposition, depth scheduling, metrics,
   and OpenQASM 3 emission — all consuming a single canonical SSA-wiring
   representation for physical qubit identity. Neutral-atom targets extract an
   **atom-indexed interaction graph** (ADR-0029, #318), schedule entangling
   layers, plan movement, compact the schedule, and build a resource report.

This is the stable high-level shape. Individual internal pass ordering remains
an implementation detail.

## Typechecker architecture

The typechecker is decomposed into judgment modules, each owning one
sub-language of the type system (ADR-0028, #207):

| Module | Judgment | ADR |
|---|---|---|
| `typecheck/circuit.rs` | Circuit typing — gate placement, depth/width inference, Clifford class | ADR-0028 |
| `typecheck/monad.rs` | Quantum Monad / borrow — `run` blocks, borrow escape policy, cleanup (measure/reset/discard per #180) | ADR-0031 |
| `typecheck/obligation.rs` | Refinement / Z3 — depth upper-bound, width equality, well-founded termination, assumption recording | ADR-0032 |
| `typecheck/classical.rs` | Classical Γ — arithmetic, lists, lambdas, branches, match/exhaustiveness, patterns, subsumption | ADR-0035 |
| `typecheck/linear.rs` | Linear Δ — resource bookkeeping (consumption, splitting, residual joining) | — |
| `typecheck/exhaust.rs` | Exhaustiveness — match-arm usefulness algorithm | — |
| `typecheck/unify.rs` | Unification — metavariable substitution table | — |

The `TypeChecker` struct in `mod.rs` is the facade that dispatches into these
modules. Each module uses `pub(super)` methods so no new public exports leak.
This decomposition means removing any one judgment (e.g. Z3) touches only its
own module — the deletion test from the typechecker split epic (#207).

## Circ extract/rebuild seam

ZX simplification (#75) and Clifford+T optimization (#96) share a faithful
**circ extract/rebuild seam** (ADR-0033, #320): `quantum.circ` regions are
extracted into a Melior-free `CircGate` IR keyed by the gate registry
(`quon_core::gates`), with SSA-root wire tracking for faithful multi-qubit
handling. Unsupported constructs **fail closed** — the rewrite is declined and
the function is left unchanged, never silently dropping gates that would change
semantics.

## Fixed physical layout

The Fixed physical path is owned by one module (`mlir_bridge/src/fixed_physical.rs`,
ADR-0034, #316). **SSA wiring is the single canonical channel** for physical
qubit identity — both OpenQASM emission and depth scheduling consume it.
`phys_qubit` attributes are a derived view written from SSA after routing, not
a primary channel. This eliminates the prior fork where emit and scheduling
could disagree on physical qubit identity.

## Neutral-atom architecture

The neutral-atom backend (`quon_na`) uses an **atom-indexed interaction graph**
(ADR-0029, #318): `InteractionGraph<V>` is generic over the vertex type, with
`V = LogicalQubitId` for the bare-qubit path and `V = AtomVertexId` for the
hybrid QEC path. The hybrid path names physical atoms directly via the
`AtomVertexId` newtype — no numeric casts from atom indices to logical qubit
IDs.

Both bare-qubit and hybrid QEC paths share a single **`plan_backend`** entry
point for placement, AOD movement, and entangling (ADR-0036, #317). The hybrid
shell (per-round expansion, Wait barriers, serial Z-then-X split, shared layout
across rounds) stays in `qec_schedule` per ADR-0016. The QEC path now populates
`NaStats` with per-stage timings, unblocking `--emit-na-stats` for QEC-backed
programs (#307).

## Target-dependent stages

Fixed targets supply:

- physical qubit count and connectivity for routing;
- native gate set for decomposition and OpenQASM emission;
- coherence and timing metadata for scheduling and metrics;
- dynamic-circuit capability flags for target reporting.

With no `--target`, the compiler uses `generic_openqasm`: a built-in 64-qubit,
all-to-all fixed target with the standard gate set.

Neutral-atom targets supply:

- zone and array geometry;
- AOD movement and Rydberg interaction parameters;
- operation timing and fidelity data;
- resource and cost-model parameters.

Those targets use `--emit-na-schedule` and `--emit-resource-report` rather than
`--emit-qasm`. `--emit-na-stats` is now supported for both bare-qubit and
QEC-backed programs.

## Diagnostics and debug checks

Parse, type, elaboration, lowering, and emission failures stop the compile and
produce diagnostics. `--dump-ir` exposes selected pipeline checkpoints on
standard error. `--verify-linear` adds debug linearity checks before and after
dynamic lowering.

The compiler still builds the target artifact when no output flag is supplied.
Output flags control what is written to standard output or files; metrics flags
report data collected from the same compile.

Useful inspection commands:

```bash
cargo run -p quonc -- --list-passes
cargo run -p quonc -- test/verify/bell.qn --dump-ir --emit-qasm
cargo run -p quonc -- \
  --target targets/neutral_atom/generic_rna_v0.json \
  --print-target
```

See the [quonc CLI reference](../quonc/) for command examples and the
[maturation path](/guides/roadmap/) for the production-hardening direction.
