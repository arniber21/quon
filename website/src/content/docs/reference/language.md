---
title: Language reference
description: Normative reference for the Quon source language — lexical syntax, types and kinds, expressions, circuits, quantum effects, built-ins, and the gate catalog.
---

This page is the **stable lookup surface** for the Quon source language. Where
the [language guide](/language/introduction/) teaches concepts progressively and
the [cookbook](/cookbook/) walks through complete programs, this reference
gives the normative form of each construct: its syntax, its typing and semantic
contract, the constraints the typechecker enforces, and a minimal valid
example. It is the page to reach for when you know *what* you want to write and
need to confirm *how* it is spelled and *what* the compiler proves about it.

The reference tracks the compiler implementation. The authoritative formal
specification lives in
[`SPEC.md`](https://github.com/arniber21/quon/blob/main/SPEC.md); this page
mirrors it as a browsable, anchored document. Discrepancies between this page
and `SPEC.md` are bugs in this page.

## Status and version

| Field | Value |
|---|---|
| Reference version | 0.1 |
| Language version | 0.1.0-draft |
| Status | Implementation reference |
| Lowering targets | OpenQASM 3.0 (fixed gate-model); neutral-atom schedule/resource outputs |

The reference describes the language **as the compiler accepts it today**. Each
section marks constructs as **implemented** (parsed, typechecked, lowered, and
exercised by the test suite) or **limited/planned** where the implementation is
incomplete or deliberately restricted. The split is summarized in
[Implementation status](#implementation-status) at the end of this page.

## Lexical structure

Source files are UTF-8. Identifiers are ASCII. Whitespace is insignificant
except as a separator.

### Identifiers

```text
ident ::= [A-Za-z_][A-Za-z0-9_]*
```

Type variables, gate names, and value names share one identifier namespace;
disambiguation is positional. Identifiers beginning with `_` are intentional
discards / wildcards.

### Keywords

```text
fn  type  let  in  return  match  circuit  run  borrow
for  if  then  else  true  false  adjoint  controlled  par
```

These are reserved and may not be used as identifiers.

### Literals

```text
int_lit   ::= [0-9]+
float_lit ::= [0-9]+ '.' [0-9]+ (['e''E'] ['+' '-']? [0-9]+)?
bool_lit  ::= 'true' | 'false'
```

`int_lit` doubles as a type-level `Nat` literal in type position. `Unit` is
written `()`.

### Operators and punctuation

| Token | Meaning |
|---|---|
| `\|>` | Sequential circuit composition (left-associative) |
| `<-` | Monadic bind in `run { }` blocks |
| `@` | Gate targeting / circuit application |
| `->` | Unrestricted function type / lambda arrow |
| `-o` | Linear function type (consumes argument) |
| `*` | Arithmetic multiplication; `par { } * n` n-fold tensor |
| `^` | Exponentiation |
| `=` | Definition |
| `:` | Type annotation |
| `,` | Tuple / parameter separator |
| `_` | Wildcard / intentional discard |
| `` ` `` | Infix combinator application (e.g. ``tensored``, ``on_high``) |

`|>` and `<-` are the only multi-character symbolic tokens.

### Comments

```kotlin
-- a single-line comment
{- a block comment -}
{- block comments {- can nest -} like this -}
```

Block comments nest. Leading doc comments on declarations are captured as
documentation (ADR-0010).

## Types and kinds

### Kinds

```text
Kind k ::=
  | Type          -- kind of value types
  | Nat           -- type-level natural numbers
  | Class         -- Clifford classification labels
  | CodeFamily    -- closed QEC code-family tags (Repetition, Surface)
```

Kinded parameters may appear on `fn` and `type` declarations. A bare parameter
defaults to kind `Nat` (`type Oracle<n> = ...`); explicit kinds are written
`<F: CodeFamily, d: Nat>`. `CodeFamily` is a closed builtin set — user code may
be generic over `F` but cannot declare new families.

```kotlin
fn memory_rounds<F: CodeFamily, d: Nat>(b: QecBlock<F, d>, rounds: Int): Q<QecBlock<F, d>> = run {
    after <- memory_round(b)
    return after
}
```

### Type grammar

```text
Type τ ::=
  | Qubit                          -- linear quantum register element
  | QReg<n>                        -- linear qubit register, n : Nat
  | Bit                            -- classical measurement result
  | Bool | Int | Float | Unit      -- unrestricted classical scalars
  | List<τ>                        -- unrestricted list
  | (τ₁, τ₂, ..., τₙ)             -- tuple
  | τ₁ -> τ₂                      -- unrestricted function
  | τ₁ -o τ₂                      -- linear function (consumes τ₁)
  | Circuit<n, m, d, C>            -- unitary circuit morphism
  | Q<τ>                           -- quantum monad
  | Matrix<n, m, τ>                -- n×m matrix of type τ
  | QecBlock<F, d>                 -- linear encoded logical qubit

Nat n ::=
  | 0 | 1 | 2 | ... | n₁ + n₂ | n₁ * n₂ | n₁ ^ n₂
  | n₁ - n₂  (n₁ ≥ n₂)            -- bounded subtraction
  | x | e                          -- type-level variable / promoted Int

Class C ::= Clifford | Universal
```

**Contract.** `Qubit`, `QReg<n>`, `Circuit<...>`, and `QecBlock<F, d>` are
*linear*: values of these types live in the linear context Δ and must be
consumed exactly once. `Bit`, `Bool`, `Int`, `Float`, `Unit`, `List<τ>`, tuples
of unrestricted types, and `τ₁ -> τ₂` are *unrestricted*: they live in Γ and may
be copied and discarded freely. `Q<τ>` is the quantum monad (see
[Quantum effects](#quantum-effects)).

**Constraints.**

- `QReg<n>` requires `n : Nat` statically known; `QReg<2>` and `QReg<3>` are
  distinct types.
- `Circuit<n, m, d, C>` requires `n, m, d : Nat` and `C : Class`.
- `QecBlock<F, d>` requires `F : CodeFamily` and `d : Nat`.
- A single program entrypoint may use QEC builtins *or* bare `Qubit`/`QReg`
  ops, not both.

### The circuit type

`Circuit<n, m, d, C>` is the central type. It denotes a unitary quantum
morphism that consumes exactly `n` qubits, produces exactly `m` qubits, has
gate depth bounded above by `d`, and has Clifford classification `C`. `n` and
`m` are independent — a circuit may widen (`encode: Circuit<1, 3, ...>`) or
narrow.

```kotlin
fn encode(): Circuit<1, 3, 2, Clifford> = circuit {
    CNOT @(0, 1) |> CNOT @(0, 2)
}
```

**Composition rules** (type-level arithmetic — these are *inference* rules that
synthesize the tight result type bottom-up):

| Operation | Result type |
|---|---|
| `f: Circuit<a,b,d₁,C₁>` \|> `g: Circuit<b,c,d₂,C₂>` | `Circuit<a, c, d₁+d₂, C₁⊔C₂>` |
| `f: Circuit<a,b,d₁,C₁>` `par` `g: Circuit<c,d,d₂,C₂>` | `Circuit<a+c, b+d, max(d₁,d₂), C₁⊔C₂>` |
| `adjoint(f: Circuit<n,m,d,C>)` | `Circuit<m, n, d, C>` |
| `controlled(f: Circuit<n,m,d,C>)` | `Circuit<n+1, m+1, d+1, C>` |
| `repeat(k, f: Circuit<n,n,d,C>)` | `Circuit<n, n, k*d, C>` |

The Clifford join `C₁ ⊔ C₂` is `Universal` if either operand is `Universal`,
else `Clifford`.

**Subtyping.** Checking an expression against an annotated circuit type uses a
single subtyping rule:

```text
Circuit<n₁,m₁,d₁,C₁>  <:  Circuit<n₂,m₂,d₂,C₂>
   iff   n₁ = n₂  ∧  m₁ = m₂      (WIDTH invariant — exact interface)
       ∧ d₁ ≤ d₂                  (DEPTH covariant — "bounded above by d")
       ∧ C₁ ⊑ C₂                  (CLASS covariant — Clifford ⊑ Universal)
```

Width is exact (no subtyping on `n`/`m`). Depth and class are covariant: a
*looser* depth annotation than the synthesized depth is accepted; a *tighter*
one is a depth mismatch. `Clifford ⊑ Universal`, so a Clifford circuit
satisfies a `Universal` annotation but not vice versa.

```kotlin
fn loose(): Circuit<1, 1, 5, Clifford> = circuit { H @0 }
```

**Symbolic depth.** When `d` contains a runtime variable (e.g. `steps * 2`),
the typechecker emits Z3 constraints at composition boundaries to verify
`δ_syn ≤ δ_ann` (see [Depth bounds](#depth-bounds)). Non-linear depth
expressions such as `n^p` (with `p` a variable) are rejected — the user must
supply a static bound.

### Depth bounds

Every `Circuit` type carries a proven upper bound on gate depth, the third
index `d`. Depth is tracked in the type, not measured at run time.

| Composition | Depth rule |
|---|---|
| Sequential (`\|>`) | `depth(a \|> b) = depth(a) + depth(b)` |
| Parallel (`par`) | `depth(par { a, b }) = max(depth(a), depth(b))` |
| Repeat (`repeat`) | `depth(repeat(n, c)) = n * depth(c)` |
| Controlled | `depth(controlled(c)) = depth(c) + 1` |

The typechecker infers a *synthesized* depth `δ_syn` bottom-up, then checks
`δ_syn ≤ δ_ann` against the declared bound. Structural equality is tried
first, then algebraic simplification (`n * 1 = n`, `max(n, n) = n`), then Z3.
If Z3 cannot prove the inequality — or times out — the bound is conservatively
rejected; the typechecker never accepts a depth it cannot prove.

```kotlin
fn evolution(steps: Nat): Circuit<1, 1, steps * 2, Universal> =
    repeat(steps, circuit { H @0 |> T @0 })
```

**Constraints.** A too-tight bound is a `DepthMismatch`. Natural subtraction
(`n - 1`) only appears in width/count expressions within a valid domain
(typically a `match n ≥ 1` arm); it never appears in a depth-of-subcircuit
position, which keeps the Monotonicity Lemma (looser annotations stay sound
when composed) valid.

### Clifford classification

`C` is `Clifford` or `Universal`, inferred bottom-up from the gate primitives.
A circuit is `Clifford` iff built entirely from Clifford gates; a single
non-Clifford gate makes the composite `Universal`.

**Constraints.** The classification is *inferred*, not annotated: declaring
`Clifford` while using `T` is a `classification mismatch`. The optimizer
dispatches on the classification — stabilizer tableau for `Clifford`, phase
polynomial for `Universal` — so the type-level fact drives the backend.

```kotlin
fn basis_change(): Circuit<1, 1, 1, Clifford> = circuit { H @0 }
fn phase_resource(): Circuit<1, 1, 1, Universal> = circuit { T @0 }
```

### Linearity

Quon enforces a linear type discipline via a split-context bidirectional
typechecker. Every expression is checked under two contexts:

- **Γ** — unrestricted classical environment (`Bit`, `Bool`, `Int`, `Float`,
  `Unit`, `List`, function types). Reusable any number of times.
- **Δ** — linear context (`Qubit`, `QReg`, `QecBlock`, `Circuit` values).
  Every binding must be **consumed exactly once**.

A linear resource is consumed when it is measured, passed to a circuit,
returned, discarded, reset, or re-tensored. After consumption the name is gone
from Δ; any subsequent reference is a compile-time error. The absence of
contraction makes no-cloning a type error:

```kotlin
-- ERROR: q used twice — violates linearity
fn clone(q: Qubit): (Qubit, Qubit) =
    let (q1, q2) = (q, q)
    in (q1, q2)
```

**Branching residuals.** When a computation branches, each arm sees a clone of
Δ; at the merge, both arms must leave the *same* set of resources live
(`LinearBranchMismatch` otherwise). **Lambda capture rejection:** linear
resources cannot be captured by closures (closures live in Γ and may be called
many times). Discard with `_` is permitted only for a measured `Qubit` or a
`Qubit` returned to `|0⟩` by a `borrow` terminator; all other discards are
errors.

## Expressions and statements

### Declarations

```text
fn name(p₁: τ₁, ..., pₖ: τₖ): τ = body
type Name<p₁: k₁, ...> = τ
```

A `fn` declaration binds a name to a function. Parameters may be linear
(tracked in Δ) or unrestricted (tracked in Γ). A `type` declaration is a
type alias. Bare parameters default to kind `Nat`.

```kotlin
fn hadamard_layer(n: Nat): Circuit<n, n, 1, Clifford> =
    par { circuit { H @0 } } * n

type Bell = Circuit<2, 2, 2, Clifford>
```

**Constraints.** A `fn` returning `Circuit<...>` with a recursive body must
terminate via a decreasing `Nat` measure (see
[Recursive circuit functions](#recursive-circuit-functions)). A program has a
single `main` entrypoint.

### `let` / `in`

```text
let x = e₁ in e₂              -- unrestricted binding (Γ)
let (x₁, ..., xₙ) = e₁ in e₂  -- destructuring / tensor elimination
```

`let` introduces a name into Γ (unrestricted) or, for a linear
destructure, moves ownership into Δ. Inside `run { }`, `let` is the
classical-binding form; `<-` is the monadic/linear-binding form (see
[Quantum effects](#quantum-effects)).

```kotlin
fn reverse_pair(q: QReg<2>): QReg<2> =
    let (left, right) = destructure(q)
    in (right, left)
```

### `if` / `then` / `else`

```text
if b then e₁ else e₂
```

**Contract.** Branches on a `Bit` (a measured quantum outcome) or a `Bool` (a
pure classical value). Both arms must produce the same type and leave the same
linear resources live. A `Bit`-conditioned `if` lowers to `quantum.dynamic.if`
(both branches present); a `Bool`-conditioned `if` on a known constant may be
folded at compile time. Only `Nat` `match` arms introduce refinement
assumptions — a `Bool`/`Bit` `if`-guard introduces none.

```kotlin
fn conditional_gate(b: Bit, q: Qubit): Q<Qubit> = run {
    result <- (if b then circuit { X @0 } else circuit { I @0 }) @ q
    return result
}
```

### `match`

```text
match scrutinee { p₁ => e₁ | ... | pₙ => eₙ }
```

**Contract.** `match` over a `Nat` scrutinee refines it per arm: a literal arm
`k =>` assumes `scrut = k`; a wildcard/variable arm `_ =>` assumes `scrut ≠ kᵢ`
for every sibling literal. Combined with the global `v ≥ 0` domain, a `{0, _}`
match yields `scrut ≥ 1` in the `_` arm — exactly what licenses `scrut − 1` as
a predecessor. Assumptions scope to the arm body and never leak. Arms must be
exhaustive.

```kotlin
fn qft(n: Nat): Circuit<n, n, 2 * n * n, Universal> =
    match n {
        0 => identity(0),
        _ => apply_hadamard(n)
             |> controlled_rotations(n)
             |> (qft(n - 1) `on_high` n)
             |> swap_reverse(n)
    }
```

### `for` loops

```text
for x in iter { body }
```

**Contract.** `for` appears inside `circuit { }` to elaborate gate layers.
Iterators: `qubits(n)` (each qubit), `range(k)` (0..k-1), `pairs(n)` (ordered
pairs i≠j), `diag(n)` (diagonal indices). Depth of a `for` is `max` when bodies
act on disjoint qubits (parallel) and `sum` when sequential — determined by
data-dependency analysis on qubit indices. The elaborator unrolls `for` at
concrete call sites.

```kotlin
fn hadamard_all(n: Nat): Circuit<n, n, 1, Clifford> = circuit {
    for q in qubits(n) { H q }
}
```

### Lambdas

```text
fn(x: τ) -> e        -- unrestricted function (lives in Γ)
```

**Constraints.** Closures are the unrestricted `Fn` kind: they may not capture
linear resources (see [Linearity](#linearity)). To thread a qubit through a
higher-order structure, use a `Circuit` value or a `run` block instead.

```kotlin
fn add_one(x: Int): Int -> Int = fn(y: Int) -> x + y
```

### Recursive circuit functions

A recursive circuit function `f(n: Nat): Circuit<φ(n), ψ(n), δ(n), C>` is
checked with its own signature in scope as the **inductive hypothesis**; a call
`f(e)` is typed by value-substitution, never by inlining the body (so the
checker cannot loop). Soundness rests on two obligations:

1. **Well-foundedness** — a decreasing `Nat` measure: some parameter `p` such
   that every recursive call, under its refinement assumptions, supplies an
   argument provably `< p` and `≥ 0`. The measure is inferred by trying each
   `Nat` parameter; if none decreases on all calls, the function is rejected
   with an *ill-founded recursion* error.
2. **Inductive depth/width/class** — the base arm discharges under refinement;
   the step arm discharges `δ_syn(step) ≤ δ(n)` via the Monotonicity Lemma.

**Constraints (limited).** v1 supports direct self-recursion with a single
inferred decreasing measure. **Mutual recursion among circuit functions is
rejected** (no per-body termination witness). A recursive call guarded by a
statically-unrefined branch is rejected; the documented workaround is to
restructure under a `match`. Classical mutual recursion (e.g. `even`/`odd`) is
unaffected.

## Circuits

A `circuit { }` block is a *pure value* of type `Circuit<n, m, d, C>`. It
performs no allocation or measurement; the optimizer may rewrite it freely. It
is the only place unitary gates are placed.

### The `circuit { }` block

```text
circuit { gate_placement (|> gate_placement)* }
```

**Contract.** Evaluates to a `circuit_val(G)` — a gate DAG. The block is
typechecked against its declared `Circuit<n, m, d, C>` annotation: width
matches the declared `n`/`m`, the synthesized depth fits `d`, and the inferred
classification fits `C`.

```kotlin
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}
```

### Gate placement `@`

```text
GATE @pos               -- single-qubit gate
GATE @(i, j)            -- two-qubit gate
(PARAM angle) @pos      -- parameterized rotation
```

**Contract.** `@` places a gate onto qubit *positions* — zero-based indices
into the circuit's input register. Positions are checked statically against the
circuit's `n`: `H @5` inside a two-qubit circuit is a type error
(`gate position out of range`). Parameterized rotations wrap the angle in
parentheses before the position: `(Rz theta) @0`.

```kotlin
fn rz_gate(theta: Float): Circuit<1, 1, 1, Universal> = circuit {
    (Rz theta) @0
}
```

### Sequential composition `|>`

```text
c₁ |> c₂
```

**Contract.** "Do this, then that." The left circuit's output width must equal
the right circuit's input width (`3 ≠ 2` is a type error). Depths add; the
classification is the join. Left-associative.

```kotlin
fn prepare_one(): Circuit<1, 1, 2, Universal> = circuit {
    H @0 |> T @0
}
```

### Parallel composition `par`

```text
par { c₁, c₂ }            -- different circuits on disjoint qubits
par { c } * k              -- k-fold tensor of c with itself
```

**Contract.** Tensor-products circuits on disjoint qubit sets. Widths add;
depth is `max` of the parts; classification is the join. `par { c } * k`
repeats `c` on `k` disjoint slices — width `k*n`, depth unchanged. Indices
inside a `par` arm are *relative* to that arm's slice.

```kotlin
fn hadamard_layer(n: Nat): Circuit<n, n, 1, Clifford> =
    par { circuit { H @0 } } * n

fn mixed_layer(): Circuit<3, 3, 2, Universal> = circuit {
    par {
        H @0 |> T @0,
        H @1
    }
}
```

### `repeat`

```text
repeat(k, c)
```

**Contract.** Runs `c` sequentially `k` times. Requires `c` square
(`Circuit<n, n, ...>`). Depth is `k * depth(c)`; class preserved.

```kotlin
fn echo(k: Nat): Circuit<1, 1, k * 3, Clifford> =
    repeat(k, circuit { H @0 |> S @0 |> H @0 })
```

### `adjoint` and `controlled`

```text
adjoint(c)        controlled(c)
```

**Contract.** `adjoint(c)` returns the unitary inverse — `Circuit<n,m,d,C> →
Circuit<m,n,d,C>` (depth unchanged, class preserved). `controlled(c)` adds a
control qubit — `Circuit<n,m,d,C> → Circuit<n+1,m+1,d+1,C>` (depth +1, class
preserved). Both are typed `Circuit` values and compose with everything else.

```kotlin
fn decode(): Circuit<3, 1, 2, Clifford> = adjoint(encode())
fn with_control(c: Circuit<1, 1, d, Clifford>): Circuit<2, 2, d + 1, Clifford> =
    controlled(c)
```

### Register reshape combinators

```text
destructure(q)   split(k, q)   a `tensored` b   (a, b)
```

| Combinator | Type | Description |
|---|---|---|
| `destructure(q)` | `QReg<n> -o (Qubit, ..., Qubit)` | Flatten into n qubits |
| `split(k, q)` | `(Nat, QReg<n>) -o (QReg<k>, QReg<n-k>)` | Split at position k (`k ≤ n`) |
| `a `tensored` b` | `QReg<n> -o QReg<m> -o QReg<n+m>` | Concatenate registers |
| `(a, b)` | tensor introduction | Combines qubits/registers into one register |

**Constraints.** There is **no register indexing** (`reg[1]`): it would alias
a linear resource. `destructure`, `split`, and `tensored` are the only ways to
change a register's shape, and each *moves* ownership rather than aliasing it.

```kotlin
fn take_first(q: QReg<5>): (QReg<2>, QReg<3>) =
    let (head, tail) = split(2, q)
    in (head, tail)
```

## Quantum effects

Dynamic work — allocation, measurement, classical feed-forward — lives in the
**Quantum Monad** `Q<T>`, written in a `run { }` block. This is the only place
allocation and measurement may happen; inside `circuit { }` they are forbidden.
The boundary is what lets the optimizer rewrite pure regions algebraically
while preserving measurement ordering on the dynamic side.

### The `run { }` block

```text
run { stmt ; ... ; stmt }
```

**Contract.** Desugars to monadic bind chains. A `run { }` block has type
`Q<T>` and must end by producing a value of type `T`. The three statement
forms are monadic bind (`<-`), classical `let`, and `return`.

```kotlin
fn hello_bell(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0       <- measure(q0)
    b1       <- measure(q1)
    return (b0, b1)
}
```

### Monadic bind `<-`

```text
x <- e
```

**Contract.** Sequences a quantum computation and binds its (classical) result
on the left. The right-hand side must produce a value in `Q<...>`. The qubits
it consumes are tracked in Δ. `<-` is *not* assignment — it is monadic bind.
`let` puts a name in Γ (unrestricted); `<-` puts a name in Δ (linear, must be
consumed).

```kotlin
fn chain(): Q<Bit> = run {
    q1 <- prepare_one() @ qubit()
    r  <- measure(q1)
    return r
}
```

### `return`

```text
return v
```

**Contract.** Lifts a classical value `v` into `Q<T>`, ending the computation.
At every `return`, the typechecker checks Δ is empty — any live linear resource
is a "linear resource not consumed" error.

### Circuit application `@` (monadic)

```text
c @ qubits
```

**Contract.** The bridge between the pure circuit world and the effectful
monadic world — the only way to execute a circuit. Applies a
`Circuit<n, m, ...>` to a `QReg<n>` (or tuple of `Qubit`s), consuming the input
and producing fresh output qubits. The typing rule:

```text
c : Circuit<n, m, d, C>    reg : QReg<n>
------------------------------------------ @
        c @ reg : Q<QReg<m>>
```

```kotlin
fn apply_and_measure(): Q<Bit> = run {
    reg <- bell_state() @ qreg(2)
    (q0, q1) = reg
    b <- measure(q0)
    discard(q1)
    return b
}
```

### Allocation

| Primitive | Type | Description |
|---|---|---|
| `qreg(n)` | `Nat -> Q<QReg<n>>` | Allocate n fresh qubits in `\|0⟩` |
| `qubit()` | `Q<Qubit>` | Allocate a single fresh qubit in `\|0⟩` |
| `init_one()` | `Q<Qubit>` | Allocate in `\|1⟩` |
| `init_plus()` | `Q<Qubit>` | Allocate in `\|+⟩ = H\|0⟩` |

**Constraints.** Allocation is monadic — it may only appear in `run { }`.

```kotlin
fn alloc(): Q<Qubit> = run {
    q <- qubit()
    return q
}
```

### Measurement

| Primitive | Type | Description |
|---|---|---|
| `measure(q)` | `Qubit -o Q<Bit>` | Destructive Z-basis measurement |
| `measure_x(q)` | `Qubit -o Q<Bit>` | X-basis (applies H first) |
| `measure_y(q)` | `Qubit -o Q<Bit>` | Y-basis measurement |
| `measure_all(q)` | `QReg<n> -o Q<List<Bit>>` | Sequential Z-basis readout of all qubits |

**Contract.** `measure` consumes a `Qubit` from Δ and produces a classical
`Bit` in Γ — copyable, reusable, branchable. After `measure(q)`, `q` is gone;
a second `measure(q)` is an unbound reference. `measure_all` is all-or-nothing
— to measure some qubits, `split` first.

`Bit` vs `Bool`: a `Bit` is an irreversible measurement outcome (random); a
`Bool` is a deterministic classical value. `if` accepts both, but a
`Bit`-conditioned branch has a genuine runtime dependency the optimizer must
preserve.

```kotlin
fn read_all(reg: QReg<4>): Q<List<Bit>> = run {
    bits <- measure_all(reg)
    return bits
}
```

### `reset` and `discard`

| Primitive | Type | Description |
|---|---|---|
| `reset(q)` | `Qubit -o Q<Qubit>` | Measure and reprepare to `\|0⟩` |
| `discard(q)` | `Qubit -o Q<Unit>` | Consume without a classical result (ancilla cleanup only) |

**Constraints.** `discard` is valid only for ancilla cleanup (e.g. inside a
`borrow` block). Both fully consume the qubit.

### Classical control (feed-forward)

```text
if bit then c₁ else c₂ @ q
```

**Contract.** Branches on a `Bit` and applies a different circuit per outcome.
Both branches must consume the same qubit(s) (the branching residual rule).
Lowers to `quantum.dynamic.if` with both branches present; the optimizer never
collapses them. On hardware without mid-circuit measurement, the
`measurement_deferral` pass rewrites feed-forward into controlled corrections.

```kotlin
fn two_bit_correction(b1: Bit, b2: Bit, q: Qubit): Q<Qubit> = run {
    q1 <- (if b1 then circuit { X @0 } else circuit { I @0 }) @ q
    q2 <- (if b2 then circuit { Z @0 } else circuit { I @0 }) @ q1
    return q2
}
```

### `borrow` blocks

```text
borrow name: Qubit in { body }
borrow (a: Qubit, b: Qubit) in { body }
borrow ws: QReg<n> in { body }
```

**Contract.** Allocates ancilla qubits for a scoped sub-computation, then
verifies at block exit that each was consumed. Three rules:

1. **Consumed before exit** — every borrowed name must be consumed (measured,
   reset, or discarded) before the block ends.
2. **No escape** — a borrowed name must not appear in the block's result.
3. **Valid cleanup** — consumption is through `measure`, `reset`, or `discard`
   (per issue #180; not only structural reset/discard).

Borrow blocks may nest; each level enforces consumption and no-escape
independently.

```kotlin
fn measured_ancilla(): Q<Bit> = run {
    borrow anc: Qubit in {
        prepared <- H @ anc
        b <- measure(prepared)
        return b
    }
}
```

## Built-ins

Built-ins are primitives provided by the runtime and standard prelude; they are
not user-definable. Gate primitives are intrinsic to the typechecker;
allocation/measurement are built into the quantum abstract machine;
combinators are part of the standard prelude.

### Circuit combinators

| Combinator | Type | Description |
|---|---|---|
| `identity(n)` | `Nat -> Circuit<n,n,0,Clifford>` | n-qubit identity |
| `adjoint(c)` | `Circuit<n,m,d,C> -> Circuit<m,n,d,C>` | Unitary inverse |
| `controlled(c)` | `Circuit<n,m,d,C> -> Circuit<n+1,m+1,d+1,C>` | Add control qubit |
| `repeat(k, c)` | `(Nat, Circuit<n,n,d,C>) -> Circuit<n,n,k*d,C>` | k-fold composition |
| `on_high(c, n)` | `(Circuit<k,k,d,C>, Nat) -> Circuit<n,n,d,C>` | Apply c to high k qubits |
| `on_low(c, n)` | `(Circuit<k,k,d,C>, Nat) -> Circuit<n,n,d,C>` | Apply c to low k qubits |
| `swap_reverse(n)` | `Nat -> Circuit<n,n,n/2,Clifford>` | Reverse qubit order |

### Iteration combinators (`circuit { }` context)

| Combinator | Description |
|---|---|
| `for q in qubits(n) { body }` | Apply body to each qubit (parallel) |
| `for i in range(k) { body(i) }` | Sequential body applications |
| `for (i,j) in pairs(n) { body }` | Each ordered pair (i,j), i≠j |
| `for i in diag(n) { body }` | Each diagonal index |
| `par { c } * k` | k-fold tensor of c |

### Monadic combinators (`run { }` context)

| Combinator | Type | Description |
|---|---|---|
| `return(v)` | `A -> Q<A>` | Lift a value into Q |
| `apply(c, q)` | `Circuit<n,m,d,C> -> QReg<n> -o Q<QReg<m>>` | Apply circuit to register |
| `map_q(f, xs)` | `(A -> Q<B>, List<A>) -> Q<List<B>>` | Monadic map (mapM) |
| `sequence_q(cs)` | `List<Q<A>> -> Q<List<A>>` | Sequence Q computations |
| `discard(q)` | `Qubit -o Q<Unit>` | Consume without result (ancilla cleanup) |

### Classical prelude

| Function | Type | Description |
|---|---|---|
| `range(n)` | `Int -> List<Int>` | `[0, 1, ..., n-1]` |
| `map(f, xs)` | `(A -> B, List<A>) -> List<B>` | Standard map |
| `fold(xs, z, f)` | `(List<A>, B, (B,A)->B) -> B` | Left fold |
| `take(n, xs)` | `(Int, List<A>) -> List<A>` | First n elements |
| `zip(xs, ys)` | `(List<A>, List<B>) -> List<(A,B)>` | Zip two lists |
| `float(n)` | `Int -> Float` | Int to Float coercion |
| `round(x)` / `sqrt(x)` / `log2(x)` | `Float -> ...` | Numeric utilities |

### Physics constants

```text
PI  : Float = 3.141592653589793
TAU : Float = 6.283185307179586   -- 2π
E   : Float = 2.718281828459045
```

### QEC builtins

`QecBlock<F, d>` is a linear resource in the `Q` monad. Constructors require an
explicit distance type argument (`repetition_code<3>()`, not bare).

| Builtin | Type | Notes |
|---|---|---|
| `repetition_code<d>()` | `Q<QecBlock<Repetition, d>>` | Z-basis init; literal `d ≥ 2` |
| `surface_code<d>()` | `Q<QecBlock<Surface, d>>` | Z-basis init; literal odd `d ≥ 3` |
| `surface_code_x<d>()` | `Q<QecBlock<Surface, d>>` | X-basis init; same distance rule |
| `memory_round(b)` | `QecBlock<F, d> → Q<QecBlock<F, d>>` | One syndrome-extraction round |
| `measure_logical_z(b)` / `measure_logical_x(b)` | `QecBlock<F, d> → Q<Bit>` | Consumes the block |
| `logical_cx(a, b)` | surface blocks → `Q<(block, block)>` | Same distance required; lowering may stub |

```kotlin
fn surface_memory(block: QecBlock<Surface, 5>): Q<QecBlock<Surface, 5>> = run {
    after <- memory_round(block)
    return after
}
```

### User-defined gates

Users may define named type aliases and parameterized gate families. User-defined
gates participate in all optimization passes; if one is provably equivalent to a
known primitive (verified by ZX rewriting), it is substituted during
optimization.

```kotlin
type Bell = Circuit<2, 2, 2, Clifford>

fn bell_gate(): Bell = circuit {
    H @0 |> CNOT @(0, 1)
}

fn phase_kickback(theta: Float): Circuit<2, 2, 3, Universal> = circuit {
    CNOT @(0, 1) |> (Rz theta) @1 |> CNOT @(0, 1)
}
```

## Gates

Quon ships a fixed catalog of gate primitives rather than letting you name
arbitrary unitaries. Each primitive has a known type and a known Clifford
classification; the fixed catalog is what lets the compiler reason about each
gate's cost and class without inspecting a matrix.

### Single-qubit gates

All single-qubit primitives have type `Circuit<1, 1, 1, C>` with `C` inferred.

| Gate | Class | Matrix |
|---|---|---|
| `I` | Clifford | `[[1,0],[0,1]]` |
| `X` | Clifford | `[[0,1],[1,0]]` |
| `Y` | Clifford | `[[0,-i],[i,0]]` |
| `Z` | Clifford | `[[1,0],[0,-1]]` |
| `H` | Clifford | `1/√2 [[1,1],[1,-1]]` |
| `S` | Clifford | `[[1,0],[0,i]]` |
| `S_dag` | Clifford | `[[1,0],[0,-i]]` |
| `T` | Universal | `[[1,0],[0,exp(iπ/4)]]` |
| `T_dag` | Universal | `[[1,0],[0,exp(-iπ/4)]]` |
| `SX` | Clifford | `1/2 [[1+i,1-i],[1-i,1+i]]` (√X) |
| `SX_dag` | Clifford | `1/2 [[1-i,1+i],[1+i,1-i]]` |

```kotlin
fn clifford_gates(): Circuit<1, 1, 1, Clifford> = circuit { S @0 }
fn universal_gate(): Circuit<1, 1, 1, Universal> = circuit { T @0 }
```

### Rotations

| Gate | Class | Matrix |
|---|---|---|
| `Rx(θ)` | Universal (θ≠kπ/2) | `[[cos θ/2, -i sin θ/2],[-i sin θ/2, cos θ/2]]` |
| `Ry(θ)` | Universal (θ≠kπ/2) | `[[cos θ/2, -sin θ/2],[sin θ/2, cos θ/2]]` |
| `Rz(θ)` | Universal (θ≠kπ/2) | `[[exp(-iθ/2),0],[0,exp(iθ/2)]]` |

`Rx(θ)`, `Ry(θ)`, `Rz(θ)` are **Clifford when `θ ∈ {0, π/2, π, 3π/2}`** — the
typechecker specializes the classification at those compile-time-constant
values. For arbitrary `θ` they are `Universal`. The angle wraps in parentheses
before the position: `(Rz theta) @0`.

```kotlin
fn rz_gate(theta: Float): Circuit<1, 1, 1, Universal> = circuit {
    (Rz theta) @0
}
```

### Two-qubit gates

All two-qubit primitives have type `Circuit<2, 2, 1, C>`.

| Gate | Class | Description |
|---|---|---|
| `CNOT` / `CX` | Clifford | Controlled-X; control @0, target @1 |
| `CY` | Clifford | Controlled-Y |
| `CZ` | Clifford | Controlled-Z (symmetric) |
| `SWAP` | Clifford | Swap two qubits |
| `iSWAP` | Clifford | iSWAP gate |
| `ECR` | Clifford | Echoed cross-resonance |
| `Rzz(θ)` | Universal | `exp(-iθ/2 Z⊗Z)` |
| `Rxx(θ)` | Universal | `exp(-iθ/2 X⊗X)` |
| `Ryy(θ)` | Universal | `exp(-iθ/2 Y⊗Y)` |
| `CRz(θ)` | Universal | Controlled-Rz |
| `CRx(θ)` | Universal | Controlled-Rx |
| `CP(θ)` | Universal | Controlled phase |

**Constraints.** `CZ` is symmetric — `CZ @(0,1)` and `CZ @(1,0)` are the same
gate — but `CNOT` is not: `CNOT @(0,1)` and `CNOT @(1,0)` produce different
unitaries. Both orderings typecheck (both are valid `(Nat, Nat)` pairs within
the width); the optimizer treats them as distinct circuits.

```kotlin
fn entangler(): Circuit<2, 2, 1, Clifford> = circuit { CNOT @(0, 1) }
fn phase_entangler(): Circuit<2, 2, 1, Clifford> = circuit { CZ @(0, 1) }
```

### Gate targeting syntax

```kotlin
CNOT @(0, 1)       -- CNOT with control qubit 0, target qubit 1
(Rz 0.5) @2        -- Rz on qubit 2
H @0               -- H on qubit 0
```

Within a `circuit { }` block over a register of size `n`, qubit indices are
`0..n-1`. Index bounds are checked statically against the circuit's `n`.

## Implementation status

This section records where the implementation is complete and where it is
limited or planned, so reference readers can tell what is enforced today.

**Implemented (parsed, typechecked, lowered, and exercised by the test
suite):**

- The full lexical structure, declaration forms, `let`/`in`, `if`/`then`/`else`,
  `match`, and `for` loops.
- The full type system: kinds, the type grammar, the `Circuit<n, m, d, C>` type
  with composition arithmetic, subtyping, symbolic depth with Z3 discharge,
  Clifford classification inference, value-dependent types, and linearity with
  branching residuals.
- The complete gate catalog (single-qubit, rotations, two-qubit) with
  compile-time index bounds and classification inference.
- `circuit { }` blocks with `@`, `|>`, `par`, `repeat`, `adjoint`,
  `controlled`, and the register-reshape combinators.
- The Quantum Monad: `run { }`, `<-`, `return`, allocation, measurement
  (Z/X/Y/`measure_all`), `reset`, `discard`, classical feed-forward, and
  `borrow` blocks with measure/reset/discard cleanup.
- QEC builtins: `repetition_code`, `surface_code`/`surface_code_x`,
  `memory_round`, `measure_logical_z`/`measure_logical_x`, with the
  bare-vs-encoded entrypoint restriction.
- Direct self-recursion of circuit functions with an inferred decreasing
  `Nat` measure.
- The standard prelude, iteration combinators, and physics constants.

**Limited / planned:**

- **Mutual recursion among circuit functions** is rejected (no per-body
  termination witness). Classical mutual recursion is unaffected. Restructure
  under a `match` as the documented workaround.
- **User-defined QEC code families** are not allowed — `CodeFamily` is a closed
  builtin set (`Repetition`, `Surface`). User code may be generic over `F`.
- **`logical_cx`** lowering may stub; it is typed and accepted but the
  backend lowering is incomplete.
- **Non-linear depth** expressions such as `n^p` (with `p` a variable) are
  rejected — supply a static bound. (`DynCircuit` is eliminated; symbolic
  linear-arithmetic depth covers variational algorithms.)
- **A recursive call guarded by a statically-unrefined branch** is rejected;
  restructure under a `match`.

## See also

- [Language guide](/language/introduction/) — the progressive, conceptual
  introduction to every construct above.
- [Cookbook](/cookbook/) — complete runnable programs exercising these
  constructs end to end.
- [`quonc` CLI reference](/reference/quonc/) — the compiler driver.
- [Compiler pipeline](/reference/compiler/) — how a typed program lowers to
  OpenQASM 3 or a neutral-atom schedule.
- [`SPEC.md`](https://github.com/arniber21/quon/blob/main/SPEC.md) — the
  authoritative formal specification this page mirrors.
