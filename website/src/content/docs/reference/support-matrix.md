---
title: Feature support matrix
description: Evidence-backed support status for every public Quon construct across parse, typecheck, OpenQASM emission, neutral-atom scheduling, and QEC scheduling, with canonical executable fragments.
sidebar:
  order: 1
---

The language guide describes what Quon *means*. This page states what Quon
*does today*, per pipeline stage, for every public construct — and pins every
claim to a checked-in executable fragment so the prose cannot silently drift
from compiler behavior.

## How to read this matrix

Each construct is graded across five pipeline stages:

- **Parse** — the lexer/parser accepts the surface syntax.
- **Typecheck** — the linear typechecker accepts the program (no diagnostic).
- **QASM** — the fixed gate-model path emits OpenQASM 3 for it.
- **NA** — the reconfigurable neutral-atom backend schedules it.
- **QEC** — the hybrid QEC neutral-atom path schedules it (where applicable).

Status symbols:

| Symbol | Status | Meaning |
|---|---|---|
| ✓ | Stable | End-to-end supported and covered by a CI-checked fixture. |
| ◐ | Partial | Supported on some stages but with a documented caveat. |
| ⚠ | Experimental | Lowers or schedules, but lacks an end-to-end CI fixture. |
| ✗ | Unsupported | Parses and/or typechecks, then fails at lowering. |
| — | N/A | Not applicable to this pipeline stage. |

### Provenance

Every ✓ row cites a canonical, CI-checked fixture:

- **OpenQASM / neutral-atom / QEC emission** — `test/lit/` FileCheck cases,
  driven in CI by `quonc/tests/lit.rs` under `QUON_REQUIRE_LIT=1` (see
  `just ci-rust`), and end-to-end `test/verify/*.qn` programs run by the
  `test/verify/*.py` verifiers in `just ci-rust`.
- **Typecheck acceptance and rejection** — `frontend/tests/{circuits,linearity,typecheck,lsp_diagnostics,lower}.rs`,
  run by `cargo nextest` in `just ci-rust`.

If a row carries a tracker link (e.g. [#368](#known-limitations)), that
limitation is open; the matrix reflects the *current* worktree state, not the
issue's original framing.

## Master matrix

### Circuits, gates, and composition

| Construct | Parse | Typecheck | QASM | NA | QEC | Notes |
|---|---|---|---|---|---|---|
| `Circuit<n, m, d, C>` value, `circuit { }` block | ✓ | ✓ | ✓ | ✓ | — | Bell fixture [↓](#circuits-and-gates) |
| Gate placement `@`, `@(i, j)` | ✓ | ✓ | ✓ | ✓ | — | Position checked against `n` |
| Sequential composition `\|>` | ✓ | ✓ | ✓ | ✓ | — | Width must match |
| Single-qubit gates `I X Y Z H S S† T T†` | ✓ | ✓ | ✓ | ✓ | — | `S_dag`/`T_dag` spellings accepted |
| `SX`, `SX†` | ✓ | ✓ | ◐ | ◐ | — | No `stdgates` keyword; decompose on emit |
| Two-qubit `CNOT CZ CY SWAP` | ✓ | ✓ | ✓ | ✓ | — | `CY`/`SWAP` via `stdgates` |
| Rotations `Rx Ry Rz` | ✓ | ✓ | ✓ | ✓ | — | Arbitrary angles → `Universal` |
| `Rzz Rxx Ryy CRz CRx CP iSWAP ECR` | ✓ | ✓ | ◐ | ◐ | — | Elaborate via decomposition; no direct `stdgates` keyword |
| `adjoint(c)` | ✓ | ✓ | ✓ | ✓ | — | `qc::adjoint` of zero-arg callees |
| `repeat(k, c)` (concrete & symbolic count) | ✓ | ✓ | ✓ | ✓ | — | Grover/QFT fixtures |
| `controlled(named_gate)` / `controlled(Rz(θ))` | ✓ | ✓ | ✓ | ✓ | — | Distributed via `decompose_controlled` |
| `controlled(par { … })` / `controlled(par { c } * k)` | ✓ | ✗ | ✗ | ✗ | — | Typechecker rejects multi-target tuple; see [#369](#known-limitations) |
| `controlled(user_parametric_circuit)` | ✓ | ✓ | ✗ | ✗ | — | Elaboration not implemented; see [#374](#known-limitations) |
| Parametric circuits (`Nat` params, `for`, `match`) | ✓ | ✓ | ✓ | ✓ | — | Specialized at call sites |

### Qubits and registers

| Construct | Parse | Typecheck | QASM | NA | QEC | Notes |
|---|---|---|---|---|---|---|
| `Qubit`, `QReg<n>` linear values | ✓ | ✓ | ✓ | ✓ | — | Width in the type |
| `qreg(n)` allocator | ✓ | ✓ | ✓ | ✓ | — | Produces `QReg<n>` of `\|0⟩` |
| `qubit()` nullary allocator | ✓ | ✓ | ✓ | ✓ | — | Single `Qubit` [↓](#qubits-and-registers) |
| `init_one()` / `init_plus()` | ✓ | ✓ | ✓ | ✓ | — | Alloc + `X`/`H`; lowered in #417, see [#368](#known-limitations) |
| Pattern destructuring `(q0, q1) <- … @ qreg(n)` | ✓ | ✓ | ✓ | ✓ | — | Idiomatic form; stable |
| Explicit `destructure(reg)` builtin | ✓ | ✓ | ✗ | ✗ | — | Typechecks; no lowering arm. Prefer pattern-bind |
| `split(k, reg)` | ✓ | ✓ | ✓ | ✓ | — | `let (hi, lo) = split(k, q)`; Shor fixture |
| `tensored` / tuple formation `(a, b)` | ✓ | ✓ | ✓ | ✓ | — | Wire-list concatenation |
| Register indexing `reg[i]` | ✗ | ✗ | ✗ | ✗ | — | Deliberately absent (no-cloning) |

### Linear type system

| Construct | Parse | Typecheck | QASM | NA | QEC | Notes |
|---|---|---|---|---|---|---|
| Linear context Δ, exact-once consumption | ✓ | ✓ | ✓ | ✓ | ✓ | Enforced before lowering |
| `measure` / circuit application / return as consumption | ✓ | ✓ | ✓ | ✓ | ✓ | |
| Double use of a qubit | ✓ | ✗ | — | — | — | Diagnostic: linear reuse [↓](#the-linear-type-system) |
| Dropping / wildcard-discard of a qubit | ✓ | ✗ | — | — | — | Diagnostic: linear unconsumed |
| Branch residual mismatch | ✓ | ✗ | — | — | — | Diagnostic: linear residual |
| Capturing a linear resource in a closure | ✓ | ✗ | — | — | — | Diagnostic: linear capture |
| `reset(q)`, `discard(q)` run-block builtins | ✓ | ✓ | ✗ | ✗ | — | Typecheck Stable; no lowering arm |

### Parallel composition and depth

| Construct | Parse | Typecheck | QASM | NA | QEC | Notes |
|---|---|---|---|---|---|---|
| `par { c } * n` | ✓ | ✓ | ✓ | ✓ | — | Unrolled by the elaborator [↓](#parallel-composition) |
| `par { c₁, c₂, … }` (`ParN`) | ✓ | ✓ | ✓ | ✓ | — | Disjoint slices; widths add |
| Symbolic depth arithmetic (`+`, `max`, `*`, `+1`) | ✓ | ✓ | ✓ | ✓ | — | Proven against declared bound |
| Depth bound too tight | ✓ | ✗ | — | — | — | Diagnostic: `DepthMismatch` |
| Intractable depth expression | ✓ | ✗ | — | — | — | Diagnostic: `DepthIntractable` |

### Clifford classification

| Construct | Parse | Typecheck | QASM | NA | QEC | Notes |
|---|---|---|---|---|---|---|
| `Clifford` / `Universal` inference | ✓ | ✓ | ✓ | ✓ | — | Bottom-up join [↓](#clifford-classification) |
| Classification mismatch (`T` annotated `Clifford`) | ✓ | ✗ | — | — | — | Diagnostic: `CliffordMismatch` |
| Stabilizer-tableau / phase-polynomial optimization | — | — | ✓ | — | — | Dispatched on `clifford=true` |

### The Quantum Monad

| Construct | Parse | Typecheck | QASM | NA | QEC | Notes |
|---|---|---|---|---|---|---|
| `Q<T>`, `run { }` block | ✓ | ✓ | ✓ | ✓ | ✓ | |
| Monadic bind `<-`, `return` | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `measure_all(reg)` | ✓ | ✓ | ✓ | ✓ | ✓ | Consumes the whole register |
| `measure_x` / `measure_y` | ✓ | ✓ | ✗ | ✗ | — | Typecheck Stable; no lowering arm |
| Non-monadic value used in `Q` position | ✓ | ✗ | — | — | — | Diagnostic: `ExpectedMonad` |

### Measurement and classical control

| Construct | Parse | Typecheck | QASM | NA | QEC | Notes |
|---|---|---|---|---|---|---|
| `measure(q)` → `Bit` | ✓ | ✓ | ✓ | ✓ | ✓ | [↓](#measurement-and-classical-control) |
| `Bit` vs `Bool` distinction | ✓ | ✓ | ✓ | ◐ | — | `Bool`-`if` may fold; `Bit`-`if` deferred |
| `if bit then C1 else C2` feed-forward | ✓ | ✓ | ✓ | ◐ | — | Deferred to coherent corrections on the fixed path; no NA fixture exercises mid-circuit feed-forward |
| Non-exhaustive `match` | ✓ | ✗ | — | — | — | Diagnostic: witness reported |
| Branch type mismatch | ✓ | ✗ | — | — | — | Diagnostic: points at offending branch |

### Borrow blocks

| Construct | Parse | Typecheck | QASM | NA | QEC | Notes |
|---|---|---|---|---|---|---|
| `borrow anc: Qubit in { … }` | ✓ | ✓ | ✗ | ✗ | ✗ | No lowering arm; see [#borrow-limitation](#known-limitations) |
| Multi-qubit / register / nested `borrow` | ✓ | ✓ | ✗ | ✗ | ✗ | Typechecked by `synth_borrow` |
| Cleanup `measure` / `reset` / `discard` (per #180) | ✓ | ✓ | — | — | — | All three accepted at typecheck |
| Ancilla not consumed at exit | ✓ | ✗ | — | — | — | Diagnostic: linear unconsumed |
| Ancilla escapes its scope | ✓ | ✗ | — | — | — | Diagnostic: `quon.borrow.escape` |

### QEC blocks

| Construct | Parse | Typecheck | QASM | NA | QEC | Notes |
|---|---|---|---|---|---|---|
| `QecBlock<F, d>` value | ✓ | ✓ | — | — | ✓ | Tracked in Δ |
| `repetition_code<d>()` constructor | ✓ | ✓ | — | — | ✓ | Lit: `repetition_d3_memory` |
| `surface_code<d>()` / `surface_code_x<d>()` | ✓ | ✓ | — | — | ✓ | Lit: `surface_d3_memory`, `surface_d3_cx` |
| `memory_round(block)` | ✓ | ✓ | — | — | ✓ | Linear; same family/distance |
| `logical_cx(a, b)` | ✓ | ✓ | — | — | ✓ | Lit: `surface_d3_cx` |
| `measure_logical_z` / `measure_logical_x` | ✓ | ✓ | — | — | ✓ | Consumes the block → `Bit` |
| `logical_t` / `logical_tdag` / `logical_ccz` | ✓ | ✓ | — | — | ⚠ | Lower + `qec_collect` for Surface; no end-to-end NA fixture; rejected on Repetition |
| Bare-vs-encoded entrypoint separation | ✓ | ✗ | — | — | — | Mixing rejected at `main` |
| Family/distance mismatch | ✓ | ✗ | — | — | — | Diagnostic: `QubitCountMismatch`-family |
| `--` line comments, `{- -}` block comments | ✓ | — | ✓ | ✓ | ✓ | [↓](#comments) |
| `//` C-style comments | ✗ | — | — | — | — | Unsupported; generic parse error, see [#372](#known-limitations) |

QEC rows are marked `—` for QASM because the QEC entrypoint lowers through the
hybrid neutral-atom path (ADR-0016), not the fixed OpenQASM path; the
[`--emit-na-schedule`](../quonc/) workflow is the supported artifact.

## Per-topic executable fragments

Each language-reference topic gets one minimal valid fragment (canonical
fixture + expected result) and, where meaningful, one intentionally invalid
fragment (canonical test + expected diagnostic family).

### Circuits and gates

**Valid** — `test/lit/emit/bell_state.qn` (CI: `quonc/tests/lit.rs`):

```kotlin
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

Expected `--emit-qasm` result (FileCheck-locked): `h q[0]; cx q[0], q[1];`
followed by two `measure` lines. The same fixture lowers to `quantum.circ`
(`test/lit/circ/bell_state_lower.qn`) and schedules on the neutral-atom
target (`test/lit/na/bell_na_mlir.qn`).

**Invalid** — `frontend/tests/circuits.rs::a_t_gate_annotated_clifford_is_rejected`:

```kotlin
fn f(): Circuit<1, 1, 1, Clifford> = circuit { T @0 }
```

Expected diagnostic family: `CliffordMismatch` — "Clifford classification
mismatch: annotated `Clifford`, inferred `Universal`".

### Qubits and registers

**Valid** — `test/lit/emit/qubit_alloc.qn`:

```kotlin
fn main(): Q<Bit> = run {
    q <- qubit()
    b <- measure(q)
    return b
}
```

Expected `--emit-qasm` result: `qubit[1] q; bit[1] c; c[0] = measure q[0];`.

**Invalid** — width mismatch (diagnostic family `GateTargetArity`, rendered
by `frontend/tests/lsp_diagnostics.rs`):

```kotlin
fn bad_apply(): Q<QReg<2>> = run {
    out <- bell_state() @ qreg(3)
    return out
}
```

Expected diagnostic family: "this gate acts on 2 qubit(s), but 3 target(s)
were given" (`GateTargetArity`). Circuit composition width mismatch uses the
related `QubitCountMismatch` family, exercised by
`frontend/tests/circuits.rs::composition_qubit_mismatch_is_reported`.

### The linear type system

**Valid** — `frontend/tests/linearity.rs::well_typed_linear_programs_are_accepted`:

```kotlin
fn f(q: QReg<2>): QReg<2> = let (a, b) = destructure(q) in (a, b)
```

Expected result: accepted (no diagnostic); both qubits re-tensored.

**Invalid** — `frontend/tests/linearity.rs::using_a_qubit_twice_points_at_the_second_use`:

```kotlin
fn f(q: Qubit): QReg<2> = (q, q)
```

Expected diagnostic family: linear reuse — "linear resource `q` already
consumed", pointing at the second use. The companion cases
`dropping_a_qubit_points_at_its_binding` (drop) and
`branch_residual_mismatch_points_at_the_offending_branch` (branch residual)
cover the other two canonical linear invariants.

### Parallel composition

**Valid** — `test/lit/emit/par_repeat.qn`:

```kotlin
fn had_one(): Circuit<1, 1, 1, Clifford> = circuit { H @0 }
fn hadamard_layer(n: Nat): Circuit<n, n, 1, Clifford> =
    par { had_one() } * n
fn main(): Q<List<Bit>> = run {
    reg <- hadamard_layer(3) @ qreg(3)
    measure_all(reg)
}
```

Expected `--emit-qasm` result: `qubit[3] q;` then `h q[0]; h q[1]; h q[2];`
—a single depth-1 layer. The disjoint-arm form `par { H @0, H @0 }` is
covered by `test/lit/emit/par_parn.qn`.

**Invalid** — depth bound too tight (diagnostic family `DepthMismatch`):

```kotlin
fn too_tight(): Circuit<1, 1, 2, Universal> = circuit {
    H @0 |> T @0 |> S @0
}
```

Expected diagnostic family: `DepthMismatch` — "circuit depth mismatch:
annotated `2`, inferred `3`". Symbolic depth the solver cannot discharge
yields `DepthIntractable` instead.

### Clifford classification

**Valid** — `frontend/tests/circuits.rs::bell_gate_type_checks_end_to_end`:

```kotlin
fn bell(): Circuit<2, 2, 2, Clifford> = circuit { H @0 |> CNOT @(0, 1) }
```

Expected result: accepted; `Clifford` inferred bottom-up from `H` and `CNOT`.
At emit time the stabilizer-tableau pass (ADR-0039) runs on the
`clifford=true` function.

**Invalid** — `frontend/tests/circuits.rs::a_t_gate_annotated_clifford_is_rejected`
(duplicate of the circuits row, restated for the classification topic):

```kotlin
fn f(): Circuit<1, 1, 1, Clifford> = circuit { T @0 }
```

Expected diagnostic family: `CliffordMismatch`.

### The Quantum Monad

**Valid** — `test/lit/emit/bell_state.qn` (`run` block):

```kotlin
fn main(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0       <- measure(q0)
    b1       <- measure(q1)
    return (b0, b1)
}
```

Expected result: the `run` block lowers to `quantum.dynamic` ops and emits
the Bell QASM above.

**Invalid** — `frontend/tests/typecheck.rs::measurement_synthesizes_the_quantum_monad`:

```kotlin
fn f(q: Qubit): Bit = measure(q)
```

Expected diagnostic family: `ExpectedMonad` — `measure(q)` lives in `Q<Bit>`,
not `Bit`, so a `Bit` return type is rejected. The well-typed twin
`fn f(q: Qubit): Q<Bit> = measure(q)` is accepted in the same test.

### Measurement and classical control

**Valid** — `test/lit/emit/teleport.qn` (feed-forward, deferred by default):

```kotlin
fn main(): Q<Bit> = run {
    (msg, alice, bob) <- prep() @ qreg(3)
    (m2, a2)          <- bell_basis() @ (msg, alice)
    x_bit             <- measure(m2)
    z_bit             <- measure(a2)
    b2                <- (if z_bit then pauli_x() else id_one()) @ bob
    b3                <- (if x_bit then pauli_z() else id_one()) @ b2
    result            <- measure(b3)
    return result
}
```

Expected `--emit-qasm` result: the `if`-feed-forward is deferred into coherent
`cx`/`cz` corrections applied before the final measurement — the FileCheck
suite asserts `CHECK-NOT: if (`. The deferral is locked by
`quonc/tests/smoke.rs::teleport_feed_forward_is_deferred_by_default`.

**Invalid** — `frontend/tests/typecheck.rs::if_branch_mismatch_points_at_offending_branch`:

```kotlin
fn f(b: Bool): Int = if b then 1 else true
```

Expected diagnostic family: branch type mismatch — the diagnostic points at
the offending (`else`) branch, not the whole `if`.

### Borrow blocks

**Valid** (typecheck only) — `frontend/tests/lsp_diagnostics.rs` borrow
fixtures:

```kotlin
fn f(): Q<Int> = run {
  borrow a: Qubit in {
    return 0
  }
}
```

Expected *typecheck* result: rejected — the borrowed `a` is not consumed at
block exit (diagnostic family: linear unconsumed, with a `linear_unconsumed_borrow_fix`
suggestion). The no-escape twin `borrow a: Qubit in { return a }` yields the
`quon.borrow.escape` diagnostic.

**Invalid** — `borrow` blocks do not yet lower. A well-typed borrow program
fails at lowering with `LowerError::Unsupported { construct: "run-block
expression" }`; there is no `Expr::Borrow` arm in `frontend/src/lower.rs` and
no `borrow` fixture in `test/lit/` or `test/verify/`. See
[Known limitations](#known-limitations).

### QEC blocks

**Valid** — `test/lit/na_qec/repetition_d3_memory_na_mlir.qn`:

```kotlin
fn repetition_d3_memory(): Q<Bit> = run {
        b <- repetition_code<3>()
        b <- memory_round(b)
        b <- memory_round(b)
        measure_logical_z(b)
}
```

Expected `--emit-na-mlir` result: a `quantum.na.schedule` over five
data+check atoms, with `memory_round`-driven syndrome extraction, `Wait`
round barriers, and a final logical-`Z` measure. The surface-code memory and
logical-`CX` paths are covered by `surface_d3_memory_na_mlir.qn` and
`surface_d3_cx_na_mlir.qn`.

**Invalid** — bare-vs-encoded mixing (entrypoint restriction):

```kotlin
fn bad_mix(): Q<List<Bit>> = run {
    block <- repetition_code(3)
    reg   <- hadamard_all(2) @ qreg(2)
    after <- memory_round(block)
    bits  <- measure_all(reg)
    return bits
}
```

Expected diagnostic family: the entrypoint-mixing check rejects the program
— an entrypoint may use QEC builtins *or* bare `Qubit`/`QReg` ops, not both.
A family/distance mismatch (e.g. passing `QecBlock<Repetition, 3>` where
`QecBlock<Surface, 5>` is expected) yields the `QubitCountMismatch`-family
"both family and distance must match" diagnostic.

### Comments

**Valid** — `--` line comments and `{- -}` nested block comments are the
supported source-comment syntax; every `test/lit/emit/*.qn` fixture uses `--`
directives and parses cleanly:

```kotlin
-- prepare a Bell pair
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)  {- trailing block comment -}
}
```

**Invalid** — `//` C-style comments are unsupported:

```kotlin
fn f(): Circuit<1, 1, 1, Clifford> = circuit {
    H @0  // not a comment
}
```

Expected diagnostic family: a generic parse error at the first `/` (the
lexer has no `//` rule). A specialized "use `--` instead" hint is tracked by
[#372](#known-limitations).

## Known limitations

The matrix surfaces these open gaps so users can tell a parse/typecheck-only
feature from an end-to-end one. Each links to its tracker; the tracker is not
the only documentation — the row above states the current behavior.

- **[#368](https://github.com/arniber21/quon/issues/368) — `init_one()`/`init_plus()` lowering.**
  The allocation-then-prep-gate lowering landed in #417 (`frontend/tests/lower.rs::init_one_lowers_to_allocation_then_x`
  and `init_plus_lowers_to_allocation_then_h`), so these builtins now emit.
  The originating tracker remains open for the residual exact state-preparation
  pipeline (#397); until that lands, treat the prep gates as the stable surface
  and the exact-prep scheduling as in progress.

- **[#369](https://github.com/arniber21/quon/issues/369) — `controlled(par { … })` target-count mismatch.**
  `controlled(c)` is typed `Circuit<k+1, k+1, …>` (one control wire), so a
  `par` body under control expects `k+1` targets, but the surface
  `@(control, target)` syntax cannot spell the nested target tuple. The
  typechecker rejects it with `GateTargetArity`; the elaborator's
  `decompose_controlled` *can* distribute control over `Par`/`ParN`, so this
  is a surface-syntax/typechecker design decision, not a lowering gap. Use
  `controlled(named_gate)` / `controlled(Rz(θ))` until resolved.

- **[#372](https://github.com/arniber21/quon/issues/372) — `//` comment diagnostic.**
  `//` is not a Quon comment; the lexer rejects it with a generic parse error
  rather than a "use `--`" hint. The supported spellings — `--` line comments
  and `{- -}` nested block comments — are stable and used throughout the
  fixture suite.

- **[#374](https://github.com/arniber21/quon/issues/374) — `controlled(user_parametric_circuit)`.**
  `controlled(c)` distributes over `Compose`/`CircuitBlock`/`Par`/`ParN`/
  `Adjoint`/named-gate/rotation arms, but a controlled wrapper around a
  *user-defined parameterized* circuit reaches elaboration and then fails
  with `elaboration is not implemented for circuit body expression`. This
  blocks Hadamard-test and phase-estimation constructions written with a
  symbolic step count; use an unrolled or named-gate form in the meantime.

- **Borrow-block lowering (no dedicated tracker).** `borrow { … }` blocks
  typecheck cleanly (consume + no-escape, per #180) but have no lowering arm
  and produce `LowerError::Unsupported { construct: "run-block expression" }`
  at lowering. Ancilla-style programs today use explicit `qreg(n)` allocation
  plus `measure`/`split`/`tensored` instead. The deepening epic
  [#201](https://github.com/arniber21/quon/issues/201) tracks post-MVP
  compiler-locality work that subsumes this.

- **Run-block `reset` / `discard` / `measure_x` / `measure_y`.** These builtins
  typecheck (registered in `frontend/src/typecheck/builtins.rs`) but have no
  lowering arm in `frontend/src/lower.rs`, so a program that calls them at the
  run-block level fails at lowering. `reset`/`discard` are currently reachable
  only inside `borrow` blocks, which themselves do not lower (see above);
  `measure_x`/`measure_y` have no end-to-end path. Prefer `measure` plus
  basis-change gates until a lowering arm lands.

- **Explicit `destructure(reg)` builtin.** Typechecks
  (`frontend/tests/linearity.rs::well_typed_linear_programs_are_accepted`) but
  has no lowering arm; the idiomatic pattern-bind form
  `(q0, q1) <- circuit @ qreg(n)` is the stable, fully-lowered alternative.

## Keeping the matrix honest

This page is a *snapshot* of the worktree at the commit it lands on. The
canonical, always-current record is the executable suite: `test/lit/` for
emission, `test/verify/` for end-to-end simulation, and `frontend/tests/` for
typecheck. When a limitation above closes, the row it touches should flip to
✓ and the tracker link should move to a "resolved" note — the fixtures, not
this prose, are the source of truth.
