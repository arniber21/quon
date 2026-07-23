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
    -- syndrome extraction, correction, etc.
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
-- Create an encoded block from physical qubits
fn repetition_code(d: Nat): QecBlock<Repetition, d>

-- Run a memory round (syndrome extraction + correction)
fn memory_round(block: QecBlock<Repetition, d>): QecBlock<Repetition, d>

-- Perform a logical CNOT between two encoded blocks
fn logical_cx(a: QecBlock<F, d>, b: QecBlock<F, d>): (QecBlock<F, d>, QecBlock<F, d>)
```

These builtins are linear: they consume their input blocks and produce
fresh output blocks, just like `Circuit` application consumes and produces
`Qubit`s.

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

## Resource expansion

When the neutral-atom backend schedules a QEC program, it expands each
`QecBlock<F, d>` into physical atom counts using per-code-family overhead
formulas. For example, a repetition code of distance `d` uses `2d + 1`
physical qubits per logical qubit. These formulas live in the architecture
model and are cited from the literature — they are not heuristic estimates.

The expansion happens at scheduling time, not at the type level. The type
system tracks logical structure; the backend translates it to physical
resources.

## Next

Now you've seen every concept in the language. The final page walks through
a complete program that uses all of them.

→ [Putting it together](../putting-together/)
