---
title: Putting it together
description: A complete Quon program that uses circuits, the Quantum Monad, measurement, classical control, and parametric elaboration.
---

You have now seen every concept in Quon's type system: circuits as values,
linear qubit ownership, depth bounds, Clifford classification, the Quantum
Monad, measurement, classical control, borrow blocks, and QEC blocks. This
page walks through a complete program that uses most of them: Grover's
search on two qubits.

## What we're building

Grover's algorithm amplifies the probability of a marked state. With two
qubits, one marked item, and one iteration, this is an **exact** special
case: the marked state is recovered with probability 1. The algorithm uses
parametric circuits, `for` loops, `repeat`, and measurement — a compact
end-to-end exercise of the language.

## The program

```kotlin
fn hadamard_all(n: Nat): Circuit<n, n, 1, Clifford> = circuit {
    for q in qubits(n) { H q }
}

fn flip_all(n: Nat): Circuit<n, n, 1, Clifford> = circuit {
    for q in qubits(n) { X q }
}

fn oracle(): Circuit<2, 2, 1, Clifford> = circuit {
    CZ @(0, 1)
}

fn diffusion(n: Nat): Circuit<n, n, 5, Clifford> = circuit {
    hadamard_all(n)
    |> flip_all(n)
    |> CZ @(0, 1)
    |> flip_all(n)
    |> hadamard_all(n)
}

fn main(): Q<List<Bit>> = run {
    reg <- (hadamard_all(2) |> oracle() |> diffusion(2)) @ qreg(2)
    results <- measure_all(reg)
    return results
}
```

## Walking through it

### The type signatures

Every function in this program carries a full `Circuit<n, m, d, C>` type.
Reading the types before the bodies tells you the shape of the computation:

- `hadamard_all(n: Nat): Circuit<n, n, 1, Clifford>` — for any width `n`,
  consumes and produces `n` qubits, depth 1 (all gates parallel), Clifford.
- `flip_all(n: Nat): Circuit<n, n, 1, Clifford>` — same shape, but with `X`
  gates instead of `H`.
- `oracle(): Circuit<2, 2, 1, Clifford>` — a fixed two-qubit circuit, depth 1,
  Clifford. The `CZ` phase-flips the `|11⟩` state.
- `diffusion(n: Nat): Circuit<n, n, 5, Clifford>` — depth 5 (five sequential
  stages), Clifford. The depth comes from five `|>`-composed circuits each of
  depth 1.
- `main(): Q<List<Bit>>` — a quantum computation that allocates qubits,
  applies a circuit, measures all outputs, and returns a list of classical
  bits.

The type signatures are not documentation — they are *proofs*. The
typechecker has proven that `hadamard_all` has depth 1 for every `n`, that
`diffusion` has depth 5, and that the full composition fits within the
declared bounds. If any of these were wrong, the program would not compile.

### Parametric circuits and elaboration

`hadamard_all` and `flip_all` take a `Nat` parameter. The type
`Circuit<n, n, 1, Clifford>` says: for any width `n`, this circuit consumes
and produces `n` qubits, runs in depth 1 (all gates parallel), and contains
only Clifford gates. The `for q in qubits(n)` loop is unrolled by the
elaborator at the call site — `hadamard_all(2)` becomes two concrete `H`
placements.

Let's trace what the elaborator produces for each call in `main`:

1. **`hadamard_all(2)`** — the elaborator substitutes `n = 2`, unrolls the
   `for` loop into `H @0` and `H @1`, and produces a `SpecializedCircuit`
   with two gate placements. The depth is `max(1, 1) = 1` — the two gates
   are on disjoint qubits, so they form a single parallel layer.

2. **`oracle()`** — already concrete (no parameters), so the elaborator
   passes it through: a single `CZ @(0, 1)` placement. Depth 1.

3. **`diffusion(2)`** — the elaborator substitutes `n = 2` in all five
   sub-circuits. `hadamard_all(2)` and `flip_all(2)` each unroll to two
   gate placements; the `CZ @(0, 1)` is already concrete. The five stages
   are composed with `|>`, so the depth is `1 + 1 + 1 + 1 + 1 = 5`.

### Sequential composition and depth

The diffusion operator chains five circuits with `|>`:

```kotlin
hadamard_all(n) |> flip_all(n) |> CZ @(0, 1) |> flip_all(n) |> hadamard_all(n)
```

Each component has depth 1, so sequential composition gives depth 5 —
exactly the declared bound `Circuit<n, n, 5, Clifford>`. The typechecker
proves this: 1 + 1 + 1 + 1 + 1 = 5 ≤ 5.

The full circuit in `main` is `hadamard_all(2) |> oracle() |> diffusion(2)`.
The total depth is `1 + 1 + 5 = 7`. The typechecker computes this from the
three components and verifies it fits the program's overall structure. Since
`main` returns `Q<List<Bit>>` (not a `Circuit`), the depth is not in the
return type — but the intermediate circuit composition is still checked
against the component bounds.

### Clifford classification

Every gate here is Clifford (`H`, `X`, `CZ`). The typechecker infers
`Clifford` bottom-up from the primitives and checks it against the declared
classification. The join lattice is simple here: `Clifford ⊔ Clifford =
Clifford` at every composition, so every function in the program is
`Clifford`. The optimizer will route this circuit through the stabilizer
tableau pass (ADR-0039).

### The Quantum Monad

`main` returns `Q<List<Bit>>` — a quantum computation that allocates qubits,
applies a circuit, measures all outputs, and returns a list of classical
bits. The `run { }` block sequences these effects:

- `qreg(2)` allocates two qubits (no circuit needed — allocation is monadic).
- `circuit @ qreg(2)` applies the composed circuit, consuming the `QReg<2>`
  and producing two `Qubit`s.
- `measure_all(reg)` measures both qubits, consuming them and producing
  `List<Bit>`.

The `<-` operator is monadic bind: it sequences each effect, binding the
classical result on the left and tracking the linear resources on the right.
The `return` lifts the `List<Bit>` into `Q<List<Bit>>`, ending the
computation.

### Linear use

The typechecker verifies that both qubits are consumed by `measure_all`.
If you forgot to measure one, the program would not compile — the linear
context Δ would still contain a live `Qubit` at the `return`.

The full linear trace:

1. `qreg(2)` introduces a `QReg<2>` into Δ.
2. `hadamard_all(2) |> oracle() |> diffusion(2)) @ qreg(2)` consumes the
   `QReg<2>` and produces a `QReg<2>` (the circuit is square: 2 in, 2 out).
3. `measure_all(reg)` consumes the `QReg<2>` and produces a `List<Bit>` in
   Γ. Δ is now empty.
4. `return results` lifts the `List<Bit>` into `Q<List<Bit>>`. Δ is empty —
   the typechecker is satisfied.

## What the optimizer does

When compiled, the Grover circuit goes through the circ fixpoint (ADR-0013),
which runs five passes in a fixed order, iterating to a fixpoint:

### 1. Gate cancellation

Peephole check for adjacent self-inverse pairs. A Hadamard is self-inverse,
so `H @0 |> H @0` cancels to the identity. In this circuit, the structure is:

```
H @0  H @1  |  CZ @(0,1)  |  H @0  H @1  X @0  X @1  |  CZ @(0,1)  |  X @0  X @1  H @0  H @1
```

The `hadamard_all |> ... |> hadamard_all` in the diffusion does not cancel
directly because the oracle and diffusion interior are between them. However,
the gate cancellation pass examines adjacency: `H @0` from `hadamard_all(2)`
at the start and `H @0` from `diffusion(2)`'s first stage *are* adjacent
(after the oracle's `CZ`, which acts on both qubits, so it does not break the
adjacency on either wire). The pass checks each wire independently — on wire
0, the sequence is `H | CZ-contrib | H`, and the `CZ` does not commute with
`H` on wire 0 alone, so the two `H` gates do not cancel. No reduction here.

### 2. Rotation merging

No rotations here (all Clifford), so this pass is a no-op.

### 3. Clifford+T optimization (stabilizer tableau)

Since the circuit is `Clifford`, the stabilizer tableau pass runs. It
simulates the circuit's action on the Pauli generators as a CHP tableau
over GF(2), then checks whether the full sequence composes to identity or a
single Pauli. For Grover with the marked state `|11⟩`, it does not — the
oracle's `CZ` is structurally distinct, and the diffusion operator is a
non-trivial reflection. The tableau pass confirms the circuit is already
minimal and makes no changes.

If the circuit had contained a redundant Clifford sequence (e.g. four `S`
gates that compose to identity, or `H·H` pairs separated by commuting
gates), the tableau pass would have detected and removed them. In this case,
the circuit is already in its minimal Clifford form.

### 4. Compiler uncomputation

This pass looks for patterns where an ancilla is computed and then uncomputed
(allocating, using, and freeing workspace). Grover's algorithm here does not
use ancillas — all qubits are data qubits — so this pass is a no-op.

### 5. ZX simplification

The bounded ZX-calculus pass applies graph rewrites to the circuit's
ZX-diagram. For a small Clifford circuit like Grover on 2 qubits, the ZX
pass may find spider-fusion or local-identity rewrites, but the stabilizer
tableau pass has already confirmed minimality. The fixpoint converges: no
pass makes a change in the second round, so the loop terminates.

### The fixpoint convergence

The five passes run in order, then the loop repeats. If no pass made a change
in a round, the loop terminates. For this circuit, the first round makes no
changes (all passes confirm the circuit is minimal), so the fixpoint converges
immediately after one round.

## The emitted QASM

After optimization, the circuit is emitted as OpenQASM 3. The Grover circuit
on two qubits produces roughly:

```qasm
OPENQASM 3.0;
include "stdgates.inc";
qubit[2] q;
bit[2] c;

// hadamard_all(2)
h q[0];
h q[1];

// oracle: phase-flip |11>
cz q[0], q[1];

// diffusion(2)
h q[0];
h q[1];
x q[0];
x q[1];
cz q[0], q[1];
x q[0];
x q[1];
h q[0];
h q[1];

// measure
c[0] = measure q[0];
c[1] = measure q[1];
```

The emitted circuit has 11 gate operations (2 H + 1 CZ + 2 H + 2 X + 1 CZ
+ 2 X + 2 H = 12 gates, but note that some `H` and `X` gates on the same
qubit may be adjacent and could be further simplified — in this exact case
they are not adjacent on the same wire). The depth is 7 (the three stages:
H-layer depth 1, oracle depth 1, diffusion depth 5). The classical
measurements happen at the end — there is no mid-circuit measurement, so the
`if`-free lowering applies directly.

## Expected result

With two qubits, one marked item (`|11⟩`), and one Grover iteration, the
marked state is recovered with probability 1. An ideal simulator should
return `11` on every shot. The `grover.py` verifier checks that the
`11` frequency exceeds 90%.

This is the exact special case: with $N = 4$ items and $M = 1$ marked item,
the optimal number of Grover iterations is $\lfloor \frac{\pi}{4} \sqrt{N/M} \rfloor = 1$,
and one iteration suffices to amplify the marked state to probability 1.
The type system does not prove this — it proves the *structural* properties
(depth, width, classification, linearity) — but the physics guarantee is a
consequence of the circuit the types describe.

## What you've seen

This program exercised:

- **Circuits as typed values** (`Circuit<n, n, d, C>`)
- **Parametric specialization** (`Nat` parameters, `for` loops, elaboration)
- **Sequential composition** (`|>`, depth adds)
- **Clifford classification** (inferred, checked, drives optimization)
- **The Quantum Monad** (`Q<List<Bit>>`, `run { }`, `<-`, `return`)
- **Linear use** (all qubits consumed by `measure_all`)
- **Measurement** (consuming qubits, producing classical bits)
- **The optimizer fixpoint** (five passes, convergence, no-op for this circuit)
- **OpenQASM emission** (the final artifact)

## Where to go next

- **[Cookbook](../cookbook/)** — deeper worked examples: teleportation,
  QFT, QAOA, Shor's kernel, and a neutral-atom QAOA schedule.
- **[Compiler internals](../../architecture/compiler-internals/)** — how
  the compiler turns this program into MLIR, optimizes it, and emits
  OpenQASM 3 or neutral-atom schedule artifacts.
- **[quonc CLI](../../reference/quonc/)** — the command-line tool that
  compiles `.qn` files.
