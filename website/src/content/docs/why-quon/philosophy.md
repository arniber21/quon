---
title: Why Quon
description: The design tradeoffs behind Quon — linear types, MLIR, Rust, and a functional source language, with links to the decisions that made each call.
---

Quon is a quantum compiler built on a wager: that the hard parts of quantum
software — resource discipline, depth accounting, target variation — belong in
the *types and the compiler*, not in the programmer's head or a runtime guard.
A CTO evaluating Quon against a Pythonic SDK or a hand-rolled QASM emitter is
really comparing three different theories of where bugs should be caught. This
page states Quon's theory plainly, names the tradeoffs it accepts, and links the
Architecture Decision Records (ADRs) that locked each choice in. Nothing here is
aspirational; every decision is implemented and testable today.

The thread running through all of it is *compile-time guarantees over runtime
checks*. Quantum resources are unusually unforgiving — you cannot clone a qubit,
you cannot forget a qubit, and measuring one destroys the superposition you
might still need. Most toolchains treat these as conventions to remember. Quon
treats them as equations the typechecker solves before a single gate is lowered
to IR. The cost is a richer type system and an opinionated language; the payoff
is that a class of runtime failures simply cannot be expressed.

## Why linear types for quantum computing?

Quantum resources carry three intrinsic constraints that classical data does
not. The no-cloning theorem forbids copying an unknown quantum state. The dual
no-dropping constraint forbids discarding an unmeasured qubit — its entanglement
with the rest of the register would leak coherence into the environment,
turning a unitary computation into a noisy channel. And measurement is
destructive: once you read a qubit, the superposition is gone and the classical
bit is what remains.

Most quantum SDKs enforce these constraints with a mix of runtime assertions,
documentation, and reviewer discipline. A program that forgets to measure an
ancilla compiles fine and fails mysteriously at execution time, or worse,
produces wrong statistics that look plausible. Quon instead enforces all three
in the type system. Every `Qubit` and `QReg<n>` is a *linear* value: it must be
consumed exactly once, and the linear context `Δ` tracks that consumption at
compile time. A program that would clone, drop, or forget to measure a qubit is
rejected before it ever runs.

Consider a Bell-pair preparation followed by measurement. The linear type of
`qreg(2)` threads through the circuit application and into two `measure` calls,
each of which consumes one qubit and yields a classical `Bit`. Nothing is left
unmeasured; nothing is used twice:

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

The same discipline that protects qubits protects the result. The `run` block
is a Quantum Monad computation; its linear bind `<-` ensures the two qubits
flow into exactly the two measurements and no further. Had the program omitted
`measure(q1)`, the typechecker would report an unconsumed linear resource
before lowering — a failure the Aer simulator would only surface as a runtime
error, or not at all. The discipline scales: the three-qubit bit-flip
`syndrome_measure` routine borrows two ancillas, consumes them through
measurement, and the borrow block's consume-based no-escape policy (per ADR-0003
as resolved by issue #180) verifies the ancillas never leak out of the block.

## Why MLIR?

MLIR is a multi-level intermediate representation infrastructure with three
properties Quon needed: pluggable dialects, region-based structuring, and a
mature, reusable optimization pass infrastructure. Building a quantum compiler
*on* MLIR rather than *beside* it means Quon inherits years of compiler
engineering — generic-form IR printing, verifiers, pass managers, region
walkers — instead of reimplementing them.

Quon ships two quantum dialects. `quantum.circ` is purely unitary: every gate
is an op over `!qubit` SSA values, and linearity is enforced at the IR level
too — each `!qubit` value has exactly one use. `quantum.dynamic` carries
measurement, feed-forward, and conditional control, with physical layout
expressed as attributes on those ops (the `quantum.physical` convention). ADR-0037
collapsed an earlier `monadic_staging` dialect directly into `quantum.dynamic`,
so the final IR has exactly two quantum dialects — a deliberate simplification
that removed an ephemeral pass whose only job was to undo its own emission.

Choosing MLIR is also a bet on the ecosystem. Quantum compilers like Quir, Qris,
and others in the MLIR quantum community share a common compilation target and
a common tooling vocabulary. Quon is a node in that graph, not a standalone
island; a pass or verifier written for one MLIR quantum dialect is legible to
the next. That legibility is worth more than the convenience of a bespoke IR.

## Why Rust?

Correctness, performance, and ergonomics, in that order. Rust's ownership model
is the structural analogue of Quon's linear type system — the same discipline
that makes Rust memory-safe makes Quon quantum-safe. When the typechecker
verifies that a `Qubit` is consumed exactly once, it is performing the same
kind of borrow-check reasoning the Rust compiler performs on a `&mut` reference.
The parallel is not merely metaphorical: the linear context `Δ` is a Rust data
structure with ownership semantics that mirror the source language's.

Rust's algebraic data types and trait system make the compiler's intermediate
representations expressive. The `DepthExpr` enum (a symbolic depth algebra of
`Nat` parameters, additions, and maxima) and the `Circuit<n, m, d, C>` index
tuple are natural fits for a typed enum + generic struct, and the trait system
lets optimization kernels be generic over gate representations without
runtime dispatch. Zero-cost abstractions mean the compiler itself is fast —
typechecking and lowering run in milliseconds on the reference corpus, which
matters when the compiler is also the LSP server.

## Why a functional source language?

Functional languages compose well, and quantum circuits are nothing if not
compositions. A `Circuit<n, m, d, C>` is a *value*: it can be passed to a
function, returned from one, stored in a list, or recursively built. The
combinators `|>` (sequential composition, where depths add) and `par`
(parallel composition, where depth is the max of the branches) are natural
expressions of circuit algebra precisely because circuits are first-class.

Recursion is first-class, which the Quantum Fourier Transform demonstrates. The
QFT is defined inductively — apply a Hadamard, a ladder of controlled
rotations, then the QFT on the remaining qubits — and Quon expresses that
induction directly, with `on_high` embedding the recursive call into the high
qubits of a larger register:

```qn
fn qft(n: Nat): Circuit<n, n, 2 * n * n, Universal> =
    match n {
        0 => identity(0),
        _ => apply_hadamard(n)
             |> controlled_rotations(n)
             |> (qft(n - 1) `on_high` n)
             |> swap_reverse(n)
    }
```

The type system is bidirectional — synthesis (reading a type bottom-up) plus
checking (pushing an expected type top-down) — which fits a functional language
better than an imperative one. Bidirectional checking lets annotations drive
inference where it matters (the `Circuit<n, m, d, C>` indices) while inferring
the rest, and it gives subtyping a clean home: `Clifford ⊑ Universal` means a
Clifford circuit is usable wherever a Universal circuit is expected, inferred
bottom-up from the gates.

## Key architectural decisions

The choices above are not free-floating; each is recorded in an ADR with its
context, considered alternatives, and consequences. These are the load-bearing
ones for understanding Quon's shape:

- **[ADR-0002: Circuit type as MLIR op attributes](https://github.com/arniber21/quon/blob/main/docs/adr/0002-circuit-type-as-op-attributes.md)** —
  the four `Circuit<n, m, d, C>` indices live as attributes on the
  `quantum.circ.func` op (with `depth` as a serialized `DepthExpr`), not as an
  MLIR parameterized type. This sidesteps thin parameterized-type support and
  keeps the symbolic depth index representable.
- **[ADR-0007: Neutral-atom schedule model](https://github.com/arniber21/quon/blob/main/docs/adr/0007-quantum-na-as-separate-dialect.md)** —
  neutral-atom placement, movement, and time-layered scheduling are a distinct
  `quantum.na` dialect, not annotations on `quantum.dynamic`, because atom
  movement and zone occupancy do not fit naturally as scalar attributes.
- **[ADR-0009: Unified `BackendTarget` kind](https://github.com/arniber21/quon/blob/main/docs/adr/0009-unified-backend-target-kind.md)** —
  a `TargetKind` enum (`Fixed` for gate-model, `NeutralAtomReconfigurable` for
  reconfigurable arrays) keeps `--target` a single concept as more
  architecture families are added, rather than a flag per architecture.
- **[ADR-0013: Clifford+T optimization order](https://github.com/arniber21/quon/blob/main/docs/adr/0013-clifford-t-opt-rm-and-tableau.md)** —
  the circ fixpoint runs `gate_cancellation` → `rotation_merging` →
  `clifford_t_opt` → `compiler_uncomputation` → `zx_simplification`, with a
  Reed–Muller phase-polynomial Universal kernel and Aaronson–Gottesman
  stabilizer tableaux for Clifford circuits.
- **[ADR-0016: Hybrid QEC + NA schedule](https://github.com/arniber21/quon/blob/main/docs/adr/0016-qec-hybrid-round-expansion.md)** —
  QEC workloads expand into `quantum.na` by per-round planners with explicit
  round barriers and a serial Z-then-X phase split, reusing the shared
  place/AOD/entangle backend rather than duplicating it.
- **[ADR-0028 through ADR-0035: Typechecker judgment decomposition](https://github.com/arniber21/quon/blob/main/docs/adr/0028-typecheck-circuit-judgment-module.md)** —
  the monolithic typechecker is carved into one deep module per judgment form:
  [Circuit](https://github.com/arniber21/quon/blob/main/docs/adr/0028-typecheck-circuit-judgment-module.md),
  [Quantum Monad/borrow](https://github.com/arniber21/quon/blob/main/docs/adr/0031-typecheck-quantum-monad-judgment-module.md),
  [refinement/Z3 obligations](https://github.com/arniber21/quon/blob/main/docs/adr/0032-typecheck-refinement-judgment-module.md),
  and
  [classical Γ](https://github.com/arniber21/quon/blob/main/docs/adr/0035-typecheck-classical-gamma-judgment-module.md).
  Each is pure code motion behind a `pub(super)` seam.
- **[ADR-0037: Collapse of the monadic staging dialect](https://github.com/arniber21/quon/blob/main/docs/adr/0037-collapse-monadic-staging.md)** —
  `run` blocks lower straight into `quantum.dynamic`, deleting an ephemeral
  staging dialect and its erasure pass; the final IR has exactly two quantum
  dialects.
- **[ADR-0038: Melior-free `SpecializedCircuit`](https://github.com/arniber21/quon/blob/main/docs/adr/0038-specialized-circuit-module.md)** —
  the typed gate DAG between elaboration and lowering has no Melior dependency,
  so specialization (partial evaluation of `Nat` parameters) is testable without
  linking LLVM.
- **[ADR-0039: Real Clifford+T optimization](https://github.com/arniber21/quon/blob/main/docs/adr/0039-real-clifford-t-optimization.md)** —
  the phase-polynomial and stabilizer-tableau algorithms of ADR-0013, shipped:
  non-adjacent T-merging through CNOTs and canonical Clifford resynthesis from
  conjugation tableaux.

## What Quon is not

Stating the boundaries is as important as stating the goals. Quon is a compiler,
and several things that look adjacent are deliberately out of scope:

- **Not a quantum simulator.** Quon emits IR and OpenQASM 3 (or NA schedules);
  simulation is delegated to Qiskit Aer and other tools through a verification
  seam. The compiler proves structure; the simulator produces statistics.
- **Not a hardware controller.** Quon compiles programs into artifacts; it does
  not drive lasers, AODs, or pulse sequences. The schedule JSON describes *what*
  should happen, not the analog commands that make it happen.
- **Not a threshold-claiming tool.** Resource reports are analytic estimates
  derived from schedule structure and target error models. They are explicitly
  not logical-threshold claims, and they are kept in separate artifacts from
  sampled failure rates so the two kinds of evidence are never fused.
- **Not a QEC decoder.** Quon's QEC support is resource accounting and
  scheduling — counting atoms, rounds, and interactions — not syndrome
  decoding. The structure Stim circuit is a sibling artifact for downstream
  sampling, not an in-compiler decoder.

## Tradeoffs made explicit

Every architectural choice closes some doors. Quon's are visible in the ADRs,
and worth naming directly here:

**Static depth bounds vs. dynamic scheduling.** Quon checks depth at compile
time, which means the `Circuit` type carries a depth bound (`DepthExpr`) but the
compiler does not adapt that bound at runtime. The tradeoff is stronger
guarantees for less runtime flexibility: you know the depth of a circuit before
you emit it, but you cannot reshape it based on a measurement outcome without
leaving the unitary `Circuit` world for the `Q` monad. The split is deliberate
— dynamic behavior lives in `run` blocks, where the type system tracks the
monadic boundary instead of pretending a circuit can be both static and
adaptive.

**Two quantum dialects vs. one.** `quantum.circ` (pure unitary) and
`quantum.dynamic` (measurement and feed-forward) are split because the
optimization stories differ: unitary circuits admit gate cancellation, rotation
merging, and Clifford+T resynthesis, while dynamic circuits need measurement
deferral and classical-region fusion. Splitting makes each optimization cleaner
but adds a lowering step. ADR-0037 collapsed a third dialect (staging) into
`dynamic`, so the final count is exactly two — the minimum that keeps the
optimization boundaries honest.

**MLIR dependency vs. standalone.** MLIR brings infrastructure but requires
LLVM. That is a real build dependency and a real reason some contributors hesitate.
Quon mitigates it with the `SpecializedCircuit` module (ADR-0038), which enables
Melior-free specialization testing: the `analyze` feature of the `frontend`
crate compiles the typechecker, elaborator, and specialization path with no
`mlir_bridge` or `melior` link. The Melior-free kernels for Clifford+T
(ADR-0039) and the circ extract/rebuild seam (ADR-0033) extend the same
principle — correctness-critical algorithms are testable without the heavyweight
IR dependency.

These tradeoffs are not regrets; they are the shape of the system. The next
page walks the compiler pipeline stage by stage, showing where each decision
lands in actual code.

→ Next: [Compiler internals](/architecture/compiler-internals/)
