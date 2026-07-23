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

-- Oracle: phase-flips |11>.
fn oracle(): Circuit<2, 2, 1, Clifford> = circuit {
    CZ @(0, 1)
}

-- Diffusion: reflect about the uniform superposition.
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

### Parametric circuits

`hadamard_all` and `flip_all` take a `Nat` parameter. The type
`Circuit<n, n, 1, Clifford>` says: for any width `n`, this circuit consumes
and produces `n` qubits, runs in depth 1 (all gates parallel), and contains
only Clifford gates. The `for q in qubits(n)` loop is unrolled by the
elaborator at the call site — `hadamard_all(2)` becomes two concrete `H`
placements.

### Sequential composition and depth

The diffusion operator chains five circuits with `|>`:

```kotlin
hadamard_all(n) |> flip_all(n) |> CZ @(0, 1) |> flip_all(n) |> hadamard_all(n)
```

Each component has depth 1, so sequential composition gives depth 5 —
exactly the declared bound `Circuit<n, n, 5, Clifford>`. The typechecker
proves this: 1 + 1 + 1 + 1 + 1 = 5 ≤ 5.

### Clifford classification

Every gate here is Clifford (`H`, `X`, `CZ`). The typechecker infers
`Clifford` bottom-up from the primitives and checks it against the declared
classification. The optimizer will route this circuit through the stabilizer
tableau pass (ADR-0039), which could detect identity sequences — though in
this case the oracle and diffusion are structurally distinct.

### The Quantum Monad

`main` returns `Q<List<Bit>>` — a quantum computation that allocates qubits,
applies a circuit, measures all outputs, and returns a list of classical
bits. The `run { }` block sequences these effects:

- `qreg(2)` allocates two qubits (no circuit needed — allocation is monadic).
- `circuit @ qreg(2)` applies the composed circuit, consuming the `QReg<2>`
  and producing two `Qubit`s.
- `measure_all(reg)` measures both qubits, consuming them and producing
  `List<Bit>`.

### Linear use

The typechecker verifies that both qubits are consumed by `measure_all`.
If you forgot to measure one, the program would not compile — the linear
context Δ would still contain a live `Qubit` at the `return`.

### What the optimizer does

When compiled, the Grover circuit goes through:

1. **Gate cancellation** — peephole check for adjacent self-inverse pairs.
   In this circuit, `hadamard_all |> ... |> hadamard_all` does not cancel
   because the oracle and diffusion are between them.
2. **Rotation merging** — no rotations here (all Clifford).
3. **Clifford+T optimization** — since the circuit is `Clifford`, the
   stabilizer tableau pass runs. It checks whether the full sequence
   composes to identity or a single Pauli. For Grover with the marked
   state `|11⟩`, it does not — the oracle's `CZ` is structurally distinct.
4. **OpenQASM emission** — the circuit is emitted as `h`, `x`, `cz` gates.

### Expected result

With two qubits, one marked item (`|11⟩`), and one Grover iteration, the
marked state is recovered with probability 1. An ideal simulator should
return `11` on every shot. The `grover.py` verifier checks that the
`11` frequency exceeds 90%.

## What you've seen

This program exercised:

- **Circuits as typed values** (`Circuit<n, n, d, C>`)
- **Parametric specialization** (`Nat` parameters, `for` loops)
- **Sequential composition** (`|>`, depth adds)
- **Clifford classification** (inferred, checked, drives optimization)
- **The Quantum Monad** (`Q<List<Bit>>`, `run { }`, `<-`, `return`)
- **Linear use** (all qubits consumed by `measure_all`)
- **Measurement** (consuming qubits, producing classical bits)

## Where to go next

- **[Cookbook](../cookbook/)** — deeper worked examples: teleportation,
  QFT, QAOA, Shor's kernel, and a neutral-atom QAOA schedule.
- **[Compiler internals](../../architecture/compiler-internals/)** — how
  the compiler turns this program into MLIR, optimizes it, and emits
  OpenQASM 3 or neutral-atom schedule artifacts.
- **[quonc CLI](../../reference/quonc/)** — the command-line tool that
  compiles `.qn` files.
