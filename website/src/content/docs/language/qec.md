---
title: QEC blocks
description: Encoded logical qubits with QEC code families, and how Quon's type system tracks them.
---

Quon is the only quantum compiler with first-class QEC types in the source
language. A `QecBlock<F, d>` is a linear resource representing an encoded
logical qubit under code family `F` at distance `d`. This is not a
post-compilation annotation — it is a type the typechecker tracks alongside
bare `Qubit` values.

## What is a QEC block?

In fault-tolerant quantum computing, logical qubits are encoded into many
physical qubits using an error-correcting code. A `QecBlock` abstracts
this encoding: it is a linear value that represents one logical qubit,
tracked by the type system, and schedulable by the neutral-atom backend.

```kotlin
fn memory_round(block: QecBlock<Repetition, 3>): QecBlock<Repetition, 3> = run {
    return block
}
```

The type `QecBlock<Repetition, 3>` carries:
- **Code family** (`Repetition`) — which QEC code encodes the logical qubit.
- **Distance** (`3`) — the code distance, determining the number of physical
  qubits and the error-correction capability.

Both are type-level parameters, checked at compile time. You cannot
accidentally mix a distance-3 block with a distance-5 protocol — the types
are incompatible.

## QEC builtins

Quon provides built-in functions for common QEC operations:

```kotlin
fn repetition_code(d: Nat): QecBlock<Repetition, d>

fn memory_round(block: QecBlock<Repetition, d>): QecBlock<Repetition, d>

fn logical_cx(a: QecBlock<F, d>, b: QecBlock<F, d>): (QecBlock<F, d>, QecBlock<F, d>)
```

These builtins are linear: they consume their input blocks and produce
fresh output blocks, just like `Circuit` application consumes and produces
`Qubit`s.

### A surface-code memory round

The surface code is the leading candidate for fault-tolerant quantum
computing. A surface-code logical qubit at distance `d` is encoded into
`d² + (d - 1)²` physical data qubits (plus ancilla qubits for syndrome
extraction). A memory round extracts the syndrome, decodes it, and applies
corrections:

```kotlin
fn surface_memory(block: QecBlock<Surface, 5>): QecBlock<Surface, 5> = run {
    after <- memory_round(block)
    return after
}
```

The type `QecBlock<Surface, 5>` tells the typechecker this is a surface-code
block at distance 5. The `memory_round` builtin consumes it and produces a
fresh block of the same type — the linear ownership is threaded forward, and
the typechecker verifies that no block is dropped between rounds.

### Logical CNOT between encoded blocks

A logical CNOT entangles two encoded blocks while preserving the code
structure. The builtin `logical_cx` consumes two blocks of the *same* family
and distance and produces two entangled blocks:

```kotlin
fn entangle_logical(a: QecBlock<Surface, 5>, b: QecBlock<Surface, 5>):
    Q<(QecBlock<Surface, 5>, QecBlock<Surface, 5>)> = run {
    (a2, b2) <- logical_cx(a, b)
    return (a2, b2)
}
```

The typechecker enforces that both blocks share the same family and distance —
a `QecBlock<Surface, 5>` and a `QecBlock<Repetition, 3>` cannot be entangled
with `logical_cx`, because the types are incompatible. This is a compile-time
guarantee, not a runtime check: the type system prevents you from mixing code
families in a single logical gate.

## The hybrid QEC path

When a Quon program uses QEC blocks, the neutral-atom backend follows a
**hybrid QEC path** (ADR-0016). This path is distinct from the bare-qubit
NA path:

1. **Round-based scheduling**: the computation is structured as rounds of
   syndrome extraction, with Wait barriers between rounds to preserve the
   shared layout across logical qubits.
2. **Serial Z-then-X split**: Pauli corrections are split into Z-type and
   X-type, applied serially within each round.
3. **Atom-indexed interaction graph**: the interaction graph for the hybrid
   path is indexed by `AtomVertexId` — a newtype distinct from
   `LogicalQubitId` — so the scheduler names physical atoms directly rather
   than casting (ADR-0029).

The hybrid round loop and Wait barriers are preserved in `qec_schedule` —
they are not collapsed into a single synthetic graph. The place/AOD/entangle
stages are shared with the bare-qubit path through `plan_backend`
(ADR-0036), so both paths use the same placement and movement code.

### Round barriers and the Z-then-X split

The round barrier is the key structural feature of the hybrid path. Between
rounds, a `Wait` barrier prevents the scheduler from compacting or reordering
operations across the boundary — each round's syndrome extraction must complete
before the next round begins. This preserves the shared atom layout across
rounds: the physical atoms that hold the data qubits stay in place, and only
the ancilla atoms (used for syndrome extraction) move.

Within a round, the Z-then-X split separates Pauli corrections into two phases:
first all Z-type corrections, then all X-type corrections. This split is made
for neutral-atom scheduling fidelity — it is explicitly *not* Stim's interleaved
extraction and not a Stim-equivalent fault-tolerant-distance claim. The
Hadamards needed to convert between Z and X bases are first-class
`local_gate` schedule actions, applied between the two phases.

### The atom-indexed interaction graph

The interaction graph for the hybrid path uses `AtomVertexId` as its vertex
type — a newtype that is distinct from the `LogicalQubitId` used in the
bare-qubit path. This means the two paths never share a call site: the
generic `InteractionGraph<V>` is instantiated with `V = AtomVertexId` for
QEC and `V = LogicalQubitId` for bare qubits, and a numeric cast between
them would not compile (ADR-0029). The scheduler names physical atoms
directly, so the schedule is grounded in the physical hardware topology.

### Bare vs. encoded: an entrypoint restriction

A program entrypoint may use QEC builtins *or* bare `Qubit`/`QReg` ops, not
both. This prevents mixing logical and physical qubits in the same
computation — a category error that would break the resource accounting.
The typechecker enforces this at the `main` function: if the body contains
both `QecBlock` operations and bare-qubit allocation (`qreg`), the program
is rejected.

```kotlin
fn bad_mix(): Q<List<Bit>> = run {
    block <- repetition_code(3)
    reg <- hadamard_all(2) @ qreg(2)
    after <- memory_round(block)
    bits <- measure_all(reg)
    return bits
}
```

```
error: mixed QEC and bare-qubit operations in entrypoint
  --> source.qn:3:18
   |
 3 |     reg <- hadamard_all(2) @ qreg(2)
   |                   ^^^^^^^^^^^^ bare qubit operation
   |
   = an entrypoint may use QEC builtins OR bare qubits, not both
   = hint: separate the bare-qubit computation into its own entrypoint
```

The restriction is a compile-time check, not a runtime one. It exists because
the two paths lower to different IR — QEC blocks go through the hybrid round
expansion (ADR-0016), while bare qubits go through the standard
`quantum.circ` + `quantum.dynamic` path. Mixing them in one entrypoint would
require the scheduler to handle both, which the architecture deliberately
avoids.

## What the type system proves

The QEC type system provides several compile-time guarantees:

- **Linear use**: every `QecBlock` is consumed exactly once, like a `Qubit`.
  You cannot forget to run a syndrome round or duplicate an encoded block.
- **Code family consistency**: you cannot mix `Repetition` blocks with
  `Surface` blocks in the same logical operation — the types are incompatible.
- **Distance matching**: a `QecBlock<Repetition, 3>` cannot be passed to a
  function expecting `QecBlock<Repetition, 5>` — the distance is a type
  parameter.
- **Bare vs. encoded separation**: a program entrypoint may use QEC
  builtins OR bare `Qubit`/`QReg` ops, not both. This prevents mixing
  logical and physical qubits in the same computation.

### A distance mismatch error

```kotlin
fn expects_d5(block: QecBlock<Surface, 5>): Q<QecBlock<Surface, 5>> = run {
    return block
}

fn bad(): Q<QecBlock<Surface, 5>> = run {
    block <- repetition_code(3)
    result <- expects_d5(block)
    return result
}
```

```
error: QEC distance mismatch
  --> source.qn:5:25
   |
 5 |     result <- expects_d5(block)
   |                         ^^^^^^
   |  expected: QecBlock<Surface, 5>
   |  got:      QecBlock<Repetition, 3>
   |
   = both family and distance must match
```

The error names the expected and actual types, including both the family and
the distance. The fix is to use a block of the correct family and distance.

## Resource expansion

When the neutral-atom backend schedules a QEC program, it expands each
`QecBlock<F, d>` into physical atom counts using per-code-family overhead
formulas. These formulas live in the architecture model and are cited from
the literature — they are not heuristic estimates.

### Per-family expansion formulas

| Code family | Physical qubits per logical | Formula |
|---|---|---|
| Repetition | `2d + 1` | linear in distance |
| Surface | `d² + (d - 1)²` | quadratic in distance |

A repetition code of distance `d` uses `2d + 1` physical qubits per logical
qubit — one data qubit and `2d` check qubits. A surface code of distance `d`
uses `d² + (d - 1)²` physical data qubits (the two sublattices of the
checkerboard), plus ancilla qubits for syndrome extraction. The expansion
happens at scheduling time, not at the type level: the type system tracks
logical structure; the backend translates it to physical resources.

### What this means for scheduling

The resource expansion determines how many atoms the scheduler must place and
how many entangling rounds are needed per logical operation. A surface-code
logical CNOT at distance 5 expands to roughly `25 + 16 = 41` data qubits per
logical qubit, plus ancillas — so two logical qubits need ~82 data atoms, and
the scheduler must route entangling operations across that many physical
qubits. The round barriers ensure the shared layout is preserved across the
extraction rounds, so the scheduler does not need to re-place atoms between
rounds — only the ancilla atoms move.

The QEC path now populates `NaStats` with per-stage timings, so
`--emit-na-stats` works for QEC-backed programs (#307). This means you can
see the scheduling cost of each round — placement, movement, entangling,
measurement — as a breakdown, which is essential for tuning the code distance
against the hardware's coherence time.

## Next

Now you've seen every concept in the language. The final page walks through
a complete program that uses all of them.

→ [Putting it together](../putting-together/)
