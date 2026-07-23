---
title: Compiler internals
description: How the Quon compiler transforms typed source into hardware-specific artifacts.
---

The high-level pipeline on the [reference page](/reference/compiler/) lists the
stages `quonc` runs; this page goes inside them. The goal is to make the
compiler legible to someone who will read its source or extend a pass: where
each stage lives, what invariant it owns, and where the decisions that shape it
are recorded. Every stage below is implemented and exercised by the test
corpus; the ADRs cited are the authoritative record of *why* each stage looks
the way it does.

The unifying theme is that Quon pushes invariants as far *left* in the pipeline
as it can — into the typechecker where possible, into Melior-free kernels where
correctness is concentrated, and into SSA wiring where physical identity must be
unambiguous. Later stages are then free to transform aggressively because the
earlier ones made the transformations safe by construction.

## The pipeline at a glance

A Quon program travels through seven stages before it becomes a target
artifact. Each stage has a single responsibility and a clean handoff to the
next:

1. **Parse** — the Tree-sitter grammar plus a Rust parser turn `.qn` source into
   the surface AST. Syntax errors stop here.
2. **Desugar** — surface syntax (infix combinators, sugar) normalizes into the
   core AST the later stages reason about.
3. **Typecheck** — the bidirectional linear typechecker validates declarations,
   quantum ownership, symbolic depth bounds, register widths, and Clifford
   classification, discharging refinement obligations through Z3.
4. **Elaborate** — parametric circuit calls are specialized at concrete call
   sites, producing a first-order gate DAG with resolved `Circuit<n, m, d, C>`
   indices.
5. **Lower** — the elaborated DAG lowers to MLIR `quantum.circ` / `quantum.dynamic`
   generic-form IR through verified Rust builders.
6. **Optimize** — the circ fixpoint (gate cancellation, rotation merging,
   Clifford+T, compiler uncomputation, ZX) simplifies the IR, followed by
   dynamic passes and target adaptation.
7. **Emit** — fixed targets run native-gate decomposition, SABRE routing, depth
   scheduling, and OpenQASM 3 emission; neutral-atom targets extract an
   interaction graph, schedule entangling layers, plan movement, and build a
   resource report.

`--dump-ir` exposes checkpoints between these stages on standard error, and
`--verify-linear` adds debug linearity checks around the dynamic lowering. The
stable shape is the seven stages; individual pass ordering within the optimize
and emit stages is an implementation detail that the ADRs pin down where it
matters.

## Typechecker architecture

The frontend typechecker was once a single ~3900-line monolith. Epic #207
decomposed it into one deep module per judgment form, so that each form's logic
lives behind one interface and can be queried and tested as a unit. The
decomposition is pure code motion: method bodies moved verbatim, no behavior
change, all fixtures and fuzz targets stay green. A single `TypeChecker` struct
in `frontend/src/typecheck/mod.rs` remains the bidirectional facade that
dispatches into the judgment modules; the methods it dispatches into are
`pub(super)`, visible to the `typecheck` parent but not a new public export.

| Module | Judgment | What it owns |
|---|---|---|
| `circuit.rs` | Circuit typing | Gate placement, `|>`/`par`/`adjoint`/`controlled` composition, depth/width inference, Clifford class join |
| `monad.rs` | Quantum Monad / borrow | `run` blocks, `<-` bind and pure-resource auto-lift, `borrow` escape + cleanup policy, monadic combinators |
| `obligation.rs` | Refinement / Z3 | Depth and width discharge, well-founded termination, `match`-arm assumption stack |
| `classical.rs` | Classical Γ | Arithmetic, lists, lambdas, `if`/`match` with exhaustiveness, pattern binding, subsumption |

The facade pattern is what makes the carve safe. Every judgment method reads
and writes the checker's shared state — the metavariable table, the refinement
assumption stack, the linear context `Δ`, the active circuit-width registers —
and calls back into the facade's generic `synth` / `check` helpers. Rather than
re-encapsulating that state behind a per-judgment context struct (churn for no
behavior gain), the methods stay on `TypeChecker` and only their *definition
site* moves. `pub(super)` is the seam: the facade reaches into the modules, and
the modules reach the facade's private fields, through Rust child-module
privacy. The four ADRs —
[ADR-0028](https://github.com/arniber21/quon/blob/main/docs/adr/0028-typecheck-circuit-judgment-module.md),
[ADR-0031](https://github.com/arniber21/quon/blob/main/docs/adr/0031-typecheck-quantum-monad-judgment-module.md),
[ADR-0032](https://github.com/arniber21/quon/blob/main/docs/adr/0032-typecheck-refinement-judgment-module.md),
[ADR-0035](https://github.com/arniber21/quon/blob/main/docs/adr/0035-typecheck-classical-gamma-judgment-module.md)
— record the deletion test each carve satisfies: removing one judgment's
concerns should require edits only to that module, not scattered across the
facade.

## SpecializedCircuit: the Melior-free boundary

Between elaboration and lowering sits a typed gate DAG with no Melior
dependency: the `SpecializedCircuit` module
([ADR-0038](https://github.com/arniber21/quon/blob/main/docs/adr/0038-specialized-circuit-module.md)).
A `SpecializedCircuit` is the elaborator's output and the lowerer's only input:
a first-order `Compose` / `GateApp` / `Adjoint` tree over concrete qubit indices
and literal angles, plus the resolved `Circuit<n, m, d, C>` indices. No
classical parameters remain — they have been partially evaluated away.

The reason this boundary matters is testability. Parametric specialization —
evaluating a `hadamard_layer(n)` call at a concrete `n`, building the gate
sequence, computing its width and depth — is pure logic with no need for an MLIR
context. Before ADR-0038, specialization and MLIR emission were one tangled
method in `lower.rs`, so specialization could only be tested linked against
LLVM/MLIR. The carve gives specialization a Melior-free home in the `frontend`
crate, gated by the `analyze` feature (`quon_core` + `z3`, no `mlir_bridge`),
so it is unit- and proptest-tested in isolation. The `full` feature adds back
only `lower`, the thin Melior adapter that emits a `SpecializedCircuit` to a
`quantum.circ.func`. The boundary is verifiably Melior-free at the build-graph
level.

## MLIR dialects

Quon lowers into exactly two quantum dialects, plus standard MLIR dialects.
The split follows a clean semantic line: pure unitary behavior versus dynamic
behavior.

**`quantum.circ` — purely unitary.** Every gate is an op over `!qubit` SSA
values, and linearity is enforced at the IR level too: each `!qubit` value has
exactly one use. The `Circuit<n, m, d, C>` indices are stored as op attributes
on `quantum.circ.func` (`in_qubits`, `out_qubits`, `depth : DepthExprAttr`,
`clifford : BoolAttr`), not as an MLIR parameterized type
([ADR-0002](https://github.com/arniber21/quon/blob/main/docs/adr/0002-circuit-type-as-op-attributes.md)).
The depth attribute carries a serialized `DepthExpr` S-expression that
optimization passes reconstruct into the Rust enum when they need to combine or
check depth bounds. Composition type-checking (that the left circuit's
`out_qubits` matches the right's `in_qubits`) happens in Rust verifier
callbacks, not in MLIR's type unifier — a consequence of keeping the symbolic
depth index representable without a custom parameterized type.

**`quantum.dynamic` — measurement and feed-forward.** This dialect carries
`measure`, `reset`, `unitary_region` (an inlined unitary island), and `if`
(conditioned on a classical bit). `run` blocks lower directly here: `qreg(n)`
becomes `n` `test.qubit` allocations, `C @ qs` becomes a `unitary_region` with
the callee's body inlined, and `if b then C₁ else C₂ @ qs` becomes a
`quantum.dynamic.if` with both branches inlined. This direct lowering is the
result of [ADR-0037](https://github.com/arniber21/quon/blob/main/docs/adr/0037-collapse-monadic-staging.md),
which collapsed an ephemeral `monadic_staging` dialect and its erasure pass. The
consequence is that circ optimization passes now descend into `unitary_region`
and `if` branch bodies to reach the gates that live there, rather than only in
`func` definitions that are dead after inlining.

## Optimization passes

The circ fixpoint runs five passes in a fixed order
([ADR-0013](https://github.com/arniber21/quon/blob/main/docs/adr/0013-clifford-t-opt-rm-and-tableau.md)):
`gate_cancellation` → `rotation_merging` → `clifford_t_opt` →
`compiler_uncomputation` → `zx_simplification`, iterating to a fixpoint so that
reductions enabling further peephole cancellations are caught in later rounds.

**Gate cancellation** removes adjacent inverse pairs. A Hadamard is
self-inverse, so `H @0 |> H @0` is the identity:

```text
before:  H @0 |> H @0        after:  identity(1)
```

Likewise a `T` and its dagger cancel on the same wire:

```text
before:  T @0 |> T_dag @0    after:  identity(1)
```

These are local, adjacency-dependent peepholes. The deeper reductions come from
the Clifford+T passes.

**Phase polynomial** ([ADR-0039](https://github.com/arniber21/quon/blob/main/docs/adr/0039-real-clifford-t-optimization.md))
extracts the non-Clifford content of a `{CNOT, T, T†}` block as a sum of linear
Boolean phase terms over GF(2). Each `T`/`T†` contributes `±1` (in π/4 units)
to the parity of its qubit — a parity that may be a non-trivial XOR of input
bits after CNOTs have acted. Terms with the same parity merge, and their
coefficients sum mod 8. A coefficient of `2` is an `S` gate (Clifford, T-count
zero), so two `T` gates on the same parity through a CNOT collapse to a single
`S`:

```text
before:  T @0 |> CNOT @(0,1) |> T @0      T-count 2
after:   S @0 |> CNOT @(0,1)             T-count 0
```

The CNOT network is preserved verbatim; re-synthesis walks it maintaining
parity tracking and emits the merged coefficient's gate at the first occurrence
of each parity, eliding subsequent `T`/`T†` on that parity. The reduction needs
no adjacency — the two `T` gates are separated by a `CNOT` — which is exactly
what the peephole pass cannot see. `H`, `S`, and other gates outside the block
act as delimiters, so `T·H·T` correctly becomes two single-`T` blocks with no
spurious merge.

**Stabilizer tableau** handles the Clifford case (`clifford = true`). An
n-qubit Clifford operation is simulated via its conjugation action on Pauli
generators as a binary tableau (CHP representation, Aaronson–Gottesman 2004).
Conjugating the identity tableau through the gate sequence reveals global
structure the peephole cannot. `S` is not self-inverse, so `gate_cancellation`
will not collapse `S·S·S·S`; the tableau proves it equals the identity and
removes every gate:

```text
before:  S @0 |> S @0 |> S @0 |> S @0      after:  identity(1)
```

The same machinery catches non-adjacent identities like `H·S⁴·H = I`, where the
Hadamards are separated by four `S` gates, and reduces a sequence to a single
Pauli when it conjugates to one (e.g., `S·S → Z`). On Universal circuits, the
pass also canonizes maximal initial and final Clifford layers, and if Universal
optimization removes all `T`/`T†`, it re-infers and may flip `clifford` to
true.

## The circ extract/rebuild seam

ZX simplification and Clifford+T optimization both need the same thing: pull a
`quantum.circ.func` into a Melior-free gate+wire representation, run a kernel,
and rebuild verified `quantum.circ` ops. Before
[ADR-0033](https://github.com/arniber21/quon/blob/main/docs/adr/0033-circ-extract-rebuild-seam.md),
the ZX pass owned a private, lossy extractor that keyed qubits by operand
position (not SSA wire identity) and *silently dropped* unsupported gate names
via a `_ => {}` match arm. That is the failure mode the shared seam exists to
prevent.

The single `mlir_bridge::circ_extract` module sits between Melior and the
optimization kernels. `extract` walks a func body in order, tracking logical
wire indices through SSA values with a `WireTracker`, and produces a `CircGate`
list whose `qubits` are faithful logical wires (after `CNOT(0,1)`, a gate on
wire `1` records `[1]`, not `[0]`) and whose `name` is the canonical registry
id (`CX → CNOT`, `S† → S_dag`). `rebuild` threads fresh wires from block
arguments, applies each `CircGate` through the verified builders, and erases
the original ops only after the new ones are wired — so the IR is never left
with dangling uses.

The contract that makes the seam safe is **fail CLOSED**. `extract` returns an
error when a round-trip cannot be faithful: structural ops like `compose`,
`tensor`, `adjoint`, `controlled`, and `borrow` (which cannot be flattened
without losing semantics), gate names absent from the registry, or operand
counts that mismatch the registry arity. Consumers treat an error as *decline
the rewrite, leave the func unchanged* — never a silent drop. No gate is ever
lost. The two optimization passes call the same seam, so an operand-position bug
cannot drift between two adapters.

## Fixed physical layout

On the fixed gate-model path, physical qubit identity has to be unambiguous, or
routing, scheduling, and emission will disagree about which qubits a gate
touches. [ADR-0034](https://github.com/arniber21/quon/blob/main/docs/adr/0034-fixed-physical-layout-module.md)
settles this with one canonical channel: **SSA qubit wiring is the
authoritative representation of physical layout, and the `phys_qubit`
attribute is a derived annotation, not a second source of truth.**

Historically identity forked. SABRE routing rewrote SSA operands and inserted
SWAP gates; OpenQASM emit followed that SSA wiring and never read `phys_qubit`.
But SABRE also wrote a `phys_qubit` `I32Attr` per gate, which scheduling and
metrics folded back into their root list — creating a second channel that could
make scheduling see a spurious extra qubit on a two-qubit gate and invent false
dependencies. The deletion test confirmed the split: dropping the attribute
writes broke scheduling while QASM still worked; dropping the SSA rewiring broke
emit while the attributes lingered.

The fix makes SSA authoritative. `resolve_phys_qubits` derives identity purely
from the `WireTracker` roots when they are present (the production pipeline
after SABRE); the attribute fold-in is removed, with a fallback only for
hand-built test fixtures that annotate gates without a tracker. Because SABRE
writes SSA and emit reads SSA, they cannot disagree. The entire fixed physical
pipeline — `native_gate_decomp` → `sabre_routing` → `native_gate_decomp` →
`depth_scheduling` — lives in one module, `mlir_bridge::fixed_physical`, so
emit, T-count sampling, and metrics consume one module's result rather than
re-deriving layout from a second channel.

## The neutral-atom path

Neutral-atom targets take a different route from the same lowered IR. Where the
fixed path routes gates onto a static connectivity graph, the neutral-atom path
plans atom movement and time-layered entangling rounds against a reconfigurable
array. The schedule IR is a separate dialect, `quantum.na`, with ops for
`alloc_atom`, `place`, `move`, `entangle`, `measure`, and `layer`
([ADR-0007](https://github.com/arniber21/quon/blob/main/docs/adr/0007-quantum-na-as-separate-dialect.md)),
because atom movement and zone occupancy do not fit as scalar attributes on
`quantum.dynamic`.

The planner works over an **atom-indexed interaction graph**
([ADR-0029](https://github.com/arniber21/quon/blob/main/docs/adr/0029-atom-indexed-hybrid-interaction-graph.md)).
The graph is generic over a vertex id type: the bare-qubit path uses
`LogicalQubitId`, and the hybrid QEC path uses a distinct `AtomVertexId` newtype
so the two never share a call site — reintroducing a numeric cast between them
fails to compile. Entangling layers are scheduled by Misra–Gries / ASAP
(`schedule_entangling_layers`); placement and AOD movement (or zoned scheduling)
run through a single shared `plan_backend` function
([ADR-0036](https://github.com/arniber21/quon/blob/main/docs/adr/0036-shared-na-place-aod-entangle.md)),
so a bug fix or diagnostic improvement in the planner applies to both the bare
and hybrid pipelines.

For QEC workloads, the hybrid path is a round loop, not one synthetic graph
through the whole program
([ADR-0016](https://github.com/arniber21/quon/blob/main/docs/adr/0016-qec-hybrid-round-expansion.md)).
`quon_qec` expands a concrete physical gate/interaction graph for each logical
op or memory round; the shared place/AOD/entangle planners run *inside* a round;
and explicit round barriers plus measurement/feed-forward dependencies prevent
compaction or reordering across rounds. Surface-memory rounds use a serial
Z-then-X phase split with Hadamards as first-class `local_gate` schedule
actions — a split made for NA scheduling fidelity, explicitly not Stim's
interleaved extraction and not a Stim-equivalent fault-tolerant-distance claim.
The round-loop shell stays in `qec_schedule`; only the shallow place/move/entangle
wiring is shared, because the round barriers and shared layout across rounds
are what keep the schedule verifiable.

→ Next: [Neutral-atom model](/architecture/na-model/)
