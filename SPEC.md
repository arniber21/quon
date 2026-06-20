# Quon Language and Compiler Specification

**Version:** 0.1.0-draft  
**Status:** Implementation Reference  
**Target:** OpenQASM 3.0 / Qiskit Aer  
**Implementation:** Rust · MLIR C API · LLVM C API (entirely Rust, no C++ or CMake)

---

## Table of Contents

1. [Project Overview](#1-project-overview)
2. [Lexical Structure](#2-lexical-structure)
3. [Type System](#3-type-system)
4. [Operational Semantics](#4-operational-semantics)
5. [Standard Library and Built-ins](#5-standard-library-and-built-ins)
6. [IR Architecture](#6-ir-architecture)
7. [Optimization Passes](#7-optimization-passes)
8. [Backend Architecture](#8-backend-architecture)
9. [Emission and Runtime Integration](#9-emission-and-runtime-integration)
10. [Implementation Plan](#10-implementation-plan)
11. [Key Design Decisions](#11-key-design-decisions)
12. [Reference Algorithms](#12-reference-algorithms)
13. [Reference Literature](#13-reference-literature)

---

## 1. Project Overview

Quon is a full-stack, MLIR-based optimizing compiler for quantum computing programs. It accepts programs written in the Quon source language — a functional language with a linear type discipline, refined type-level resource tracking, and a monadic classical/quantum interface — and lowers them through a structured three-dialect MLIR stack to OpenQASM 3.0, with execution and verification via Qiskit Aer.

### 1.1 Design Goals

- Demonstrate non-trivial IR and compiler design for a non-classical computation model using MLIR's progressive lowering infrastructure.
- Enforce quantum-specific invariants statically: no-cloning (linear types), Clifford classification, and gate depth as refined type indices — each directly motivating downstream optimization passes.
- Provide a complete optimization pipeline: ZX-calculus rewriting, Clifford+T optimization, rotation merging, gate cancellation, depth-aware SABRE routing, noise-weighted scheduling, and native gate decomposition.
- Emit valid OpenQASM 3.0 and integrate with Qiskit Aer for simulation-based verification.

### 1.2 Implementation Language

| Component | Language / Interface | Rationale |
|---|---|---|
| Lexer, parser, type checker | Rust | Memory safety on complex algorithmic code |
| ZX-graph data structure | Rust | Graph algorithms benefit from Rust's ownership model |
| MLIR dialect definitions | Rust via MLIR C API | `MlirDialect`, `MlirOperationState`, `mlirDialectRegistryAddToContext`; dialect verifiers written as Rust callbacks registered with `mlirOperationStateSetAttributeVerifier` |
| MLIR optimization passes | Rust via MLIR C API | External passes registered with `mlirRegisterExternalPass`; IR walked and rewritten using `mlirOperationWalk` + `mlirOperationReplace` |
| MLIR pass manager | Rust via MLIR C API | `MlirPassManager`, `mlirPassManagerAddOwnedPass`, `mlirPassManagerRun` |
| OpenQASM emitter | Rust via MLIR C API | Walks final IR using `mlirOperationWalk`, extracts attributes with `mlirOperationGetAttributeByName` |
| Compiler driver | Rust | Single `quonc` binary; no subprocess boundary |
| Aer verification bridge | Python | Thin wrapper over `qasm3.loads()` + `AerSimulator` |

**No C++ or CMake.** All LLVM and MLIR integration goes through the stable C APIs (`libLLVM` and `libMLIR`). Rust `build.rs` links these libraries via `pkg-config` or explicit `-L`/`-l` flags. No TableGen is compiled at build time — dialect op definitions are registered programmatically through the C API at startup.

### 1.3 Name

*Quon* derives from Greenberger's quon algebra — a framework generalizing quantum statistics interpolating between bosonic and fermionic particles. The compiler binary is `quonc`.

---

## 2. Lexical Structure

### 2.1 Character Set

Source files are UTF-8. Identifiers are ASCII. The `|>` composition operator and `<-` bind operator are the only multi-character symbolic tokens.

### 2.2 Keywords

```
fn       type     let      in       return   match
circuit  run      borrow   for      in       if
then     else     true     false    adjoint  controlled
```

### 2.3 Identifiers

```
ident ::= [a-zA-Z_][a-zA-Z0-9_]*
```

Type variables and gate names share the identifier namespace; disambiguation is positional.

### 2.4 Literals

```
int_lit   ::= [0-9]+
float_lit ::= [0-9]+ '.' [0-9]+ (['e''E'] ['+' '-']? [0-9]+)?
bool_lit  ::= 'true' | 'false'
```

### 2.5 Operators and Punctuation

| Token | Meaning |
|---|---|
| `\|>` | Sequential circuit composition (left-associative) |
| `<-` | Monadic bind in `run { }` blocks |
| `@` | Gate targeting / circuit application |
| `->` | Function type / lambda arrow |
| `-o` | Linear function type (consumes argument) |
| `*` | Arithmetic multiplication; `par { } * n` for n-fold tensor |
| `^` | Exponentiation |
| `=` | Definition |
| `:` | Type annotation |
| `,` | Tuple / parameter separator |
| `_` | Wildcard / intentional linear discard |

### 2.6 Comments

```
-- single line comment
{- block comment -}
```

---

## 3. Type System

### 3.1 Kinds

```
Kind k ::=
  | Type          -- the kind of value types
  | Nat           -- the kind of type-level natural numbers
  | Class         -- the kind of Clifford classification labels
```

### 3.2 Type Grammar

```
Type τ ::=
  | Qubit                          -- linear quantum register element
  | QReg<n>                        -- linear qubit register, n : Nat
  | Bit                            -- classical measurement result
  | Bool | Int | Float | Unit      -- unrestricted classical scalars
  | List<τ>                        -- unrestricted list
  | (τ₁, τ₂, ..., τₙ)             -- tuple
  | τ₁ -> τ₂                      -- unrestricted function
  | τ₁ -o τ₂                      -- linear function (consumes τ₁)
  | Circuit<n, m, d, C>           -- unitary circuit morphism
  | Q<τ>                           -- quantum monad
  | Matrix<n, m, τ>               -- n×m matrix of type τ

Nat n ::=
  | 0 | 1 | 2 | ...               -- numeric literals
  | n₁ + n₂ | n₁ * n₂             -- arithmetic
  | n₁ ^ n₂                       -- exponentiation
  | n₁ - n₂  (n₁ ≥ n₂)           -- bounded subtraction
  | x                              -- type-level variable (from fn param)
  | e                              -- runtime Int promoted to symbolic Nat

Class C ::=
  | Clifford
  | Universal
```

### 3.3 Circuit Type

`Circuit<n, m, d, C>` is the central type. It denotes a unitary quantum morphism that:
- Consumes a register of exactly `n` qubits
- Produces a register of exactly `m` qubits
- Has gate depth bounded above by `d`
- Has Clifford classification `C`

#### Composition rules (type-level arithmetic)

| Operation | Result type |
|---|---|
| `f: Circuit<a,b,d₁,C₁>` \|> `g: Circuit<b,c,d₂,C₂>` | `Circuit<a, c, d₁+d₂, C₁⊔C₂>` |
| `f: Circuit<a,b,d₁,C₁>` par `g: Circuit<c,d,d₂,C₂>` | `Circuit<a+c, b+d, max(d₁,d₂), C₁⊔C₂>` |
| `adjoint(f: Circuit<n,m,d,C>)` | `Circuit<m, n, d, C>` |
| `controlled(f: Circuit<n,m,d,C>)` | `Circuit<n+1, m+1, d+1, C>` |
| `repeat(k, f: Circuit<n,n,d,C>)` | `Circuit<n, n, k*d, C>` |

Where the Clifford join `C₁ ⊔ C₂` is:

```
Clifford  ⊔ Clifford  = Clifford
Clifford  ⊔ Universal = Universal
Universal ⊔ Clifford  = Universal
Universal ⊔ Universal = Universal
```

#### Symbolic depth

When `d` contains a runtime variable (e.g. `p * (n*n + 1)` where `p : Int`), the type checker emits Z3 constraints at composition boundaries to verify arithmetic consistency. The depth index is a linear arithmetic expression over static `Nat` literals and runtime `Int` variables. Non-linear constraints (e.g. `d = n^p`) are rejected — the user must supply a static bound manually.

This eliminates the need for `DynCircuit` in most variational algorithms: `fold` over `p` layers of depth `d` produces `Circuit<n, n, p*d, C>` with a symbolic but well-typed depth.

### 3.4 Linearity

Quon enforces a standard linear type discipline via a **split context** bidirectional type checker.

#### Judgment forms

```
Γ ; Δ ⊢ e : τ
```

Where:
- `Γ` — unrestricted context (classical values, reusable)
- `Δ` — linear context (qubits, circuits-as-values, must be consumed exactly once)

#### Linear context rules

**Variable (linear):**
```
──────────────────────
Γ ; x:τ ⊢ x : τ        -- consumes x from Δ
```

**Variable (unrestricted):**
```
x:τ ∈ Γ
──────────────────────
Γ ; · ⊢ x : τ          -- Δ unchanged
```

**Linear function introduction:**
```
Γ ; Δ, x:τ₁ ⊢ e : τ₂
──────────────────────────────
Γ ; Δ ⊢ fn(x: τ₁) -> e : τ₁ -o τ₂
```

**Linear function elimination:**
```
Γ ; Δ₁ ⊢ f : τ₁ -o τ₂    Γ ; Δ₂ ⊢ e : τ₁
────────────────────────────────────────────
Γ ; Δ₁, Δ₂ ⊢ f e : τ₂       -- Δ₁ and Δ₂ must be disjoint
```

**Tensor introduction (QReg construction):**
```
Γ ; Δ₁ ⊢ q₁ : Qubit    Γ ; Δ₂ ⊢ q₂ : Qubit
──────────────────────────────────────────────
Γ ; Δ₁, Δ₂ ⊢ (q₁, q₂) : QReg<2>
```

**Tensor elimination (destructure):**
```
Γ ; Δ₁ ⊢ q : QReg<n>    Γ ; Δ₂, x₁:Qubit, ..., xₙ:Qubit ⊢ e : τ
────────────────────────────────────────────────────────────────────
Γ ; Δ₁, Δ₂ ⊢ let (x₁,...,xₙ) = destructure(q) in e : τ
```

**Discard (`_`):**

A linear value may be discarded with `_` only if it is:
- A `Qubit` that has been measured (post-measurement state is classical)
- A `Qubit` returned to |0⟩ by a `borrow` block terminator

All other attempts to discard a linear value are type errors.

#### No-cloning

Structural rule of contraction is absent from the linear context. The following is a type error:

```
-- ERROR: q used twice — violates linearity
fn clone(q: Qubit): (Qubit, Qubit) =
    let (q1, q2) = (q, q)  -- q appears in both Δ branches
    in (q1, q2)
```

### 3.5 The Quantum Monad

`Q<τ>` is the type of quantum computations that may perform mid-circuit measurement and return a value of type `τ`. It is a state monad threading quantum register state and classical side-effects.

`run { }` blocks are syntactic sugar for monadic bind chains. The desugaring is:

```
run {
    x <- e₁
    e₂
}
⟶  bind(e₁, fn(x) -> e₂)
```

```
run {
    let (a, b) = destructure(q)
    e
}
⟶  let (a, b) = destructure(q) in e
```

```
run {
    return v
}
⟶  return(v)
```

### 3.6 Refinement Depth Checking

At each composition boundary, the type checker collects a depth constraint and submits it to Z3. For example:

```kotlin
fn ising_evolve(n: Nat, n_steps: Int): Circuit<n, n, n_steps * n, Universal>
```

When this is composed with another circuit `g: Circuit<n, n, d₂, C>`, the checker emits:

```
assert(n_steps * n + d₂ == total_depth)
```

If `total_depth` is a type annotation on the enclosing function, Z3 verifies the equality. If it is inferred, Z3 is not needed — the expression is simply carried forward symbolically.

Z3 is invoked only when:
1. A symbolic depth expression must be shown equal to a concrete bound, or
2. A branch (e.g. `match`) produces circuits of different symbolic depths that must unify.

### 3.7 Clifford Classification Inference

Clifford classification is inferred bottom-up from gate primitives. The following gates are Clifford:

```
H, X, Y, Z, S, S†, T†(= S†·Z), CNOT, CZ, SWAP, and any circuit
composed entirely of the above
```

The following gates are Universal (non-Clifford):

```
T, Rz(θ) for θ ∉ {0, π/2, π, 3π/2}, Rx(θ), Ry(θ) for arbitrary θ,
Rzz(θ), and any circuit containing the above
```

Classification is propagated through `|>`, `par`, `adjoint`, and `controlled` using the join rule in §3.3. Users never annotate `Clifford` or `Universal` manually — it is always inferred. User-supplied annotations are checked against the inferred class and rejected if inconsistent.

### 3.8 Type Checking Algorithm

The type checker is a **bidirectional** algorithm with two modes:

- **Synthesis** (`⊢ e ⇒ τ`): infer the type of `e` bottom-up
- **Checking** (`⊢ e ⇐ τ`): verify that `e` has type `τ` top-down

Synthesis is used for: literals, variables, function application, `circuit { }` blocks.  
Checking is used for: function bodies against their declared return type, `run { }` blocks against `Q<τ>`, `match` branches against a shared type.

Circuit depth indices are synthesized at each composition point and compared symbolically. The Z3 oracle is called only as described in §3.6.

---

## 4. Operational Semantics

This section defines the small-step operational semantics of Quon, covering both the classical (unrestricted) fragment and the quantum (linear + monadic) fragment.

### 4.1 Values

```
Value v ::=
  | n                          -- integer literal
  | f                          -- float literal
  | true | false               -- boolean literals
  | ()                         -- unit
  | (v₁, ..., vₙ)              -- tuple value
  | fn(x: τ) -> e              -- function closure
  | circuit_val(G)             -- circuit value, G a gate list DAG
  | qubit_ref(i)               -- runtime qubit reference (linear)
  | qreg_val(i, n)             -- runtime qubit register (linear), starts at index i
  | bit(0) | bit(1)            -- classical bit
  | Q_val(s, e)                -- suspended quantum computation: state s, continuation e
```

### 4.2 Evaluation Contexts

```
E ::=
  | □                          -- hole
  | E e                        -- function application, evaluating function
  | v E                        -- function application, evaluating argument
  | (v₁, ..., vₙ₋₁, E, ...)   -- tuple, left-to-right
  | let x = E in e             -- let binding
  | E |> e                     -- composition, left side
  | v |> E                     -- composition, right side
  | E @ e                      -- circuit application, circuit position
  | v @ E                      -- circuit application, argument position
  | run { E }                  -- run block hole
```

### 4.3 Classical Reduction Rules

These rules govern the unrestricted (non-quantum) fragment and operate on a standard heap `H`.

**Beta reduction:**
```
(fn(x) -> e) v  ⟶  e[v/x]
```

**Let binding:**
```
let x = v in e  ⟶  e[v/x]
```

**Tuple projection:**
```
let (x₁, ..., xₙ) = (v₁, ..., vₙ) in e  ⟶  e[v₁/x₁, ..., vₙ/xₙ]
```

**Arithmetic:**
```
n₁ + n₂  ⟶  n₁+n₂    (and analogously for -, *, ^, /)
```

**Boolean:**
```
if true  then e₁ else e₂  ⟶  e₁
if false then e₁ else e₂  ⟶  e₂
```

**Match:**
```
match v { p₁ => e₁ | ... | pₙ => eₙ }  ⟶  eᵢ[σ]
    where pᵢ is the first pattern matching v, σ is the binding substitution
```

### 4.4 Circuit Reduction Rules

`circuit { }` blocks evaluate to `circuit_val(G)` where `G` is a gate DAG. The following rules describe circuit construction and composition.

**Gate application in circuit block:**
```
circuit { G; gate_name @(q₁,...,qₖ) }
  ⟶  circuit_val(G ++ [Gate(gate_name, [q₁,...,qₖ])])
```

**Sequential composition (`|>`):**
```
circuit_val(G₁) |> circuit_val(G₂)
  ⟶  circuit_val(G₁ ++ G₂)     -- append gate lists, maintaining qubit renaming
```

**Parallel composition (`par`):**
```
circuit_val(G₁) `par` circuit_val(G₂)
  ⟶  circuit_val(G₁ ⊗ G₂)     -- disjoint qubit union of gate lists
```

**Adjoint:**
```
adjoint(circuit_val(G))
  ⟶  circuit_val(reverse(G†))  -- reverse gate order, apply gate inverses
```

**Controlled:**
```
controlled(circuit_val(G))
  ⟶  circuit_val(add_control_qubit(G))
```

**Repeat:**
```
repeat(0, c)  ⟶  circuit_val(identity)
repeat(k, c)  ⟶  c |> repeat(k-1, c)
```

**For loop in circuit block:**
```
circuit { for x in range(n) { body(x) } }
  ⟶  circuit { body(0) } |> circuit { body(1) } |> ... |> circuit { body(n-1) }
```

### 4.5 Quantum Operational Semantics

The quantum fragment requires a richer machine state. We define a **quantum abstract machine** with state:

```
Machine state Σ ::= ⟨ψ, M, K⟩

where:
  ψ  : C^(2^n)           -- the current n-qubit statevector (normalized)
  M  : Nat -> QubitAddr   -- qubit allocation map: logical index -> physical slot
  K  : Continuation       -- the remaining computation (a stack of run { } frames)
```

#### Qubit allocation

```
qreg(n)  ⟶  qreg_val(i, n)
    where i = fresh_base_index(M)
    and M' = M ∪ {i ↦ fresh, i+1 ↦ fresh, ..., i+n-1 ↦ fresh}
    and ψ' = ψ ⊗ |0⟩^n        -- extend statevector with n fresh |0⟩ qubits
```

#### Circuit application (`@`)

```
⟨ψ, M, K⟩  ──  circuit_val(G) @ qreg_val(i, n)  ──▶  ⟨ψ', M, K⟩

where ψ' = U_G ψ
      U_G is the unitary matrix of G, applied to qubits M(i)...M(i+n-1)
```

The unitary `U_G` is computed by composing the matrices of each gate in `G` in order, with appropriate tensor products and qubit permutations to target the correct positions in ψ.

#### Measurement

```
⟨ψ, M, K⟩  ──  measure(qubit_ref(i))  ──▶  ⟨ψ', M', bit(b)⟩

where:
  p₀ = ‖Π₀ ψ‖²              -- probability of measuring 0 on qubit M(i)
  b  ~ Bernoulli(1 - p₀)     -- sample outcome
  ψ' = Πb ψ / ‖Πb ψ‖        -- project and renormalize
  M' = M \ {i}               -- remove qubit from allocation map (consumed)
```

`Π₀`, `Π₁` are the projectors onto the 0 and 1 subspaces for qubit `M(i)` respectively.

#### Reset

```
⟨ψ, M, K⟩  ──  reset(qubit_ref(i))  ──▶  ⟨ψ', M, qubit_ref(i)⟩

where ψ' = measure-and-reprepare qubit M(i) to |0⟩
         = (apply X if outcome was 1) after measuring
```

#### Monadic bind in `run { }` blocks

```
⟨ψ, M, K⟩  ──  x <- e₁; e₂  ──▶  ⟨ψ, M, K ∘ (x. e₂)⟩  then eval e₁
```

The continuation `K` is extended with the binding `x. e₂`. When `e₁` reduces to a value `v`, the machine steps:

```
⟨ψ', M', (x. e₂) :: K⟩  ──  v  ──▶  ⟨ψ', M', K⟩  eval e₂[v/x]
```

#### Classical conditional (feed-forward)

```
⟨ψ, M, K⟩  ──  if bit(1) then e₁ else e₂  ──▶  ⟨ψ, M, K⟩  eval e₁
⟨ψ, M, K⟩  ──  if bit(0) then e₁ else e₂  ──▶  ⟨ψ, M, K⟩  eval e₂
```

#### Borrow block

```
⟨ψ, M, K⟩  ──  borrow q: Qubit in { body }  ──▶
    ⟨ψ ⊗ |0⟩, M ∪ {j ↦ fresh}, K ∘ borrow_end(j)⟩  eval body[qubit_ref(j)/q]

-- At borrow_end(j), the verifier checks qubit j is in state |0⟩.
-- In the operational semantics this is a runtime assertion:
⟨ψ, M, borrow_end(j) :: K⟩  ──▶
    assert ⟨0|ψ_j|0⟩ = 1          -- qubit j must be |0⟩
    ⟨trace_out_j(ψ), M \ {j}, K⟩  -- discard qubit j from state
```

The static type checker (borrow block rule §3.8) guarantees this assertion holds for well-typed programs. The runtime check is a debug-mode assertion only.

#### measure_all

```
measure_all(qreg_val(i, n))
  ⟶  measure(qubit_ref(i)); x₀ <-
      measure(qubit_ref(i+1)); x₁ <-
      ...
      return [x₀, x₁, ..., xₙ₋₁]
```

Desugars to sequential measurement of each qubit. Produces `Q<List<Bit>>`.

### 4.6 Destructuring Semantics

```
destructure(qreg_val(i, n))
  ⟶  (qubit_ref(i), qubit_ref(i+1), ..., qubit_ref(i+n-1))
```

The `QReg` is consumed atomically; each resulting `Qubit` is an independent linear value referencing its physical slot in `M`.

```
split(k, qreg_val(i, n))
  ⟶  (qreg_val(i, k), qreg_val(i+k, n-k))    requires k ≤ n
```

### 4.7 Tensor Construction Semantics

Implicit tensor product from tuple syntax:

```
(qubit_ref(i), qubit_ref(j))
  ⟶  qreg_val(compact(i,j), 2)
```

Where `compact` allocates a contiguous register and inserts SWAP gates if `i` and `j` are non-adjacent in `M`. This SWAP insertion is visible in the emitted OpenQASM.

```
qreg_val(i, a) `tensored` qreg_val(j, b)
  ⟶  qreg_val(compact(i,j), a+b)
```

### 4.8 Progress and Preservation

**Theorem (Progress):** If `Γ ; Δ ⊢ e : τ` and `e` is not a value, then there exists `e'` such that `e ⟶ e'`.

**Theorem (Preservation):** If `Γ ; Δ ⊢ e : τ` and `e ⟶ e'`, then `Γ ; Δ' ⊢ e' : τ` where `Δ'` contains exactly the linear resources not yet consumed by the reduction.

**Theorem (Linearity soundness):** For any well-typed program, every `qubit_ref(i)` in `M` is referenced by exactly one linear variable in `Δ` at every point in the reduction sequence.

Proofs are by standard structural induction on the typing derivation and reduction sequence. The split-context discipline ensures no aliasing; the absence of contraction ensures no duplication.

---

## 5. Standard Library and Built-ins

The following are primitives provided by the Quon runtime and standard library. They are not user-definable — their implementations are either intrinsic to the type checker (gate primitives), built into the quantum abstract machine (allocation, measurement), or part of the standard prelude (higher-order combinators).

### 5.1 Qubit Allocation

| Primitive | Type | Description |
|---|---|---|
| `qreg(n)` | `Nat -> Q<QReg<n>>` | Allocate n fresh qubits in \|0⟩ |
| `qubit()` | `Q<Qubit>` | Allocate a single fresh qubit in \|0⟩ |

### 5.2 Register Operations

| Primitive | Type | Description |
|---|---|---|
| `destructure(q)` | `QReg<n> -o (Qubit, ..., Qubit)` | Flatten register into n-tuple of Qubits |
| `split(k, q)` | `(Nat, QReg<n>) -o (QReg<k>, QReg<n-k>)` | Split register at position k |
| `tensored(a, b)` | `QReg<n> -o QReg<m> -o QReg<n+m>` | Concatenate two registers (infix backtick) |

### 5.3 Measurement

| Primitive | Type | Description |
|---|---|---|
| `measure(q)` | `Qubit -o Q<Bit>` | Destructive single-qubit measurement in Z basis |
| `measure_x(q)` | `Qubit -o Q<Bit>` | Measurement in X basis (applies H first) |
| `measure_y(q)` | `Qubit -o Q<Bit>` | Measurement in Y basis |
| `measure_all(q)` | `QReg<n> -o Q<List<Bit>>` | Sequential Z-basis measurement of all qubits |
| `reset(q)` | `Qubit -o Q<Qubit>` | Measure and reprepare to \|0⟩ |

### 5.4 Single-Qubit Gate Primitives

All gate primitives have type `Circuit<1, 1, 1, C>` where `C` is inferred.

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
| `Rx(θ)` | Universal (θ≠kπ/2) | `[[cos θ/2, -i sin θ/2],[-i sin θ/2, cos θ/2]]` |
| `Ry(θ)` | Universal (θ≠kπ/2) | `[[cos θ/2, -sin θ/2],[sin θ/2, cos θ/2]]` |
| `Rz(θ)` | Universal (θ≠kπ/2) | `[[exp(-iθ/2),0],[0,exp(iθ/2)]]` |
| `SX` | Clifford | `1/2 [[1+i,1-i],[1-i,1+i]]` (√X) |
| `SX_dag` | Clifford | `1/2 [[1-i,1+i],[1+i,1-i]]` |

`Rx(θ)`, `Ry(θ)`, `Rz(θ)` are Clifford when `θ ∈ {0, π/2, π, 3π/2}` — the type checker specializes classification at those values.

### 5.5 Two-Qubit Gate Primitives

All two-qubit gate primitives have type `Circuit<2, 2, 1, C>`.

| Gate | Class | Description |
|---|---|---|
| `CNOT` | Clifford | Controlled-X; control @0, target @1 |
| `CX` | Clifford | Alias for CNOT |
| `CY` | Clifford | Controlled-Y |
| `CZ` | Clifford | Controlled-Z |
| `SWAP` | Clifford | Swap two qubits |
| `iSWAP` | Clifford | iSWAP gate |
| `ECR` | Clifford | Echoed cross-resonance |
| `Rzz(θ)` | Universal | `exp(-iθ/2 Z⊗Z)` |
| `Rxx(θ)` | Universal | `exp(-iθ/2 X⊗X)` |
| `Ryy(θ)` | Universal | `exp(-iθ/2 Y⊗Y)` |
| `CRz(θ)` | Universal | Controlled-Rz |
| `CRx(θ)` | Universal | Controlled-Rx |
| `CP(θ)` | Universal | Controlled phase |

### 5.6 Gate Targeting Syntax

Gates are applied to qubit positions within a `circuit { }` block using the `@` operator:

```kotlin
CNOT @(0, 1)       -- CNOT with control qubit 0, target qubit 1
Rz(0.5) @2         -- Rz on qubit 2
H @0               -- H on qubit 0
```

Within a `circuit { }` block over a register of size `n`, qubit indices are `0..n-1`. Index bounds are checked statically against the circuit's `n` parameter.

### 5.7 Circuit Combinators

| Combinator | Type | Description |
|---|---|---|
| `identity(n)` | `Nat -> Circuit<n,n,0,Clifford>` | n-qubit identity |
| `adjoint(c)` | `Circuit<n,m,d,C> -> Circuit<m,n,d,C>` | Unitary inverse |
| `controlled(c)` | `Circuit<n,m,d,C> -> Circuit<n+1,m+1,d+1,C>` | Add control qubit |
| `repeat(k, c)` | `(Nat, Circuit<n,n,d,C>) -> Circuit<n,n,k*d,C>` | k-fold composition |
| `on_high(c, n)` | `(Circuit<k,k,d,C>, Nat) -> Circuit<n,n,d,C>` | Apply c to high k qubits of n-qubit register |
| `on_low(c, n)` | `(Circuit<k,k,d,C>, Nat) -> Circuit<n,n,d,C>` | Apply c to low k qubits |
| `swap_reverse(n)` | `Nat -> Circuit<n,n,n/2,Clifford>` | Reverse qubit order |

### 5.8 Iteration Combinators (circuit { } context)

| Combinator | Type | Description |
|---|---|---|
| `for q in qubits(n) { body }` | Produces `Circuit<n,n,_,_>` | Apply body to each qubit in parallel |
| `for i in range(k) { body(i) }` | Produces `Circuit<n,n,_,_>` | Sequential body applications |
| `for (i,j) in pairs(n) { body }` | Produces `Circuit<n,n,_,_>` | Apply body to each ordered pair (i,j), i≠j |
| `for i in diag(n) { body }` | Produces `Circuit<n,n,_,_>` | Apply body to each diagonal index |
| `par { c } * k` | `Circuit<k*n, k*n, d, C>` | k-fold tensor product of c with itself |

Depth of `for` constructs is determined by whether bodies are independent (parallel → max) or sequential (→ sum). The type checker determines this from data-dependency analysis on qubit indices.

### 5.9 Monadic Combinators (`run { }` context)

| Combinator | Type | Description |
|---|---|---|
| `return(v)` | `A -> Q<A>` | Lift a value into Q |
| `apply(c, q)` | `Circuit<n,m,d,C> -> QReg<n> -o Q<QReg<m>>` | Apply circuit to register |
| `apply_dyn(c, q)` | `Circuit<n,n,d,C> -> QReg<n> -o Q<QReg<n>>` | Apply symbolic-depth circuit |
| `init_one()` | `Q<Qubit>` | Allocate qubit in \|1⟩ state |
| `init_plus()` | `Q<Qubit>` | Allocate qubit in \|+⟩ = H\|0⟩ state |
| `map_q(f, xs)` | `(A -> Q<B>, List<A>) -> Q<List<B>>` | Monadic map (mapM) |
| `sequence_q(cs)` | `List<Q<A>> -> Q<List<A>>` | Sequence a list of Q computations |
| `discard(q)` | `Qubit -o Q<Unit>` | Measure and discard result (for ancilla cleanup only) |

### 5.10 Classical Prelude

Standard functional utilities operating on unrestricted types:

| Function | Type | Description |
|---|---|---|
| `range(n)` | `Int -> List<Int>` | [0, 1, ..., n-1] |
| `map(f, xs)` | `(A -> B, List<A>) -> List<B>` | Standard map |
| `fold(xs, z, f)` | `(List<A>, B, (B,A)->B) -> B` | Left fold |
| `take(n, xs)` | `(Int, List<A>) -> List<A>` | First n elements |
| `zip(xs, ys)` | `(List<A>, List<B>) -> List<(A,B)>` | Zip two lists |
| `float(n)` | `Int -> Float` | Int to Float coercion |
| `round(x)` | `Float -> Int` | Round to nearest integer |
| `sqrt(x)` | `Float -> Float` | Square root |
| `log2(x)` | `Float -> Float` | Base-2 logarithm |

### 5.11 Physics Constants

Available as top-level bindings in scope for all programs:

```
PI    : Float = 3.141592653589793
TAU   : Float = 6.283185307179586    -- 2π
E     : Float = 2.718281828459045
```

### 5.12 User-Defined Gates

Users may define named gate aliases and parameterized gate families:

```kotlin
-- Named alias (Clifford inferred)
type Bell = Circuit<2, 2, 2, Clifford>

fn bell_gate(): Bell = circuit {
    H @0 |> CNOT @(0,1)
}

-- Parameterized gate family (Universal inferred)
fn phase_kickback(theta: Float): Circuit<2, 2, 2, Universal> = circuit {
    CNOT @(0,1) |> Rz(theta) @1 |> CNOT @(0,1)
}
```

User-defined gates participate in all optimization passes. If a user-defined gate is provably equivalent to a known primitive (verified by ZX rewriting), it is substituted during optimization.

---

## 6. IR Architecture

### 6.1 Overview

Quon's IR is a three-dialect MLIR stack. Each dialect enforces a distinct set of invariants and hosts optimization passes valid only at that abstraction level. Dialect boundaries are explicit conversion passes.

```
[ Quon Source ]
      |
      | Rust: parse → typecheck → monomorphize → build MLIR in-memory (C API)
      v
[ quantum.circ ]     Purely unitary. No measurement. Clifford and depth
                     as op attributes. ZX-calculus rewriting lives here.
      |
      | Monadic lowering pass (Rust, MLIR C API external pass)
      v
[ quantum.dynamic ]  Dynamic circuits. Measurement ops consume !qubit,
                     produce !bit. Classical SSA regions model feed-forward.
                     quantum.circ sub-circuits embedded as regions.
      |
      | Physical lowering pass (Rust, MLIR C API external pass, target descriptor applied)
      v
[ quantum.physical ] quantum.dynamic ops annotated with hardware attributes:
                     phys_qubit, native_gate, fidelity. Routing and
                     scheduling modify these attributes in place.
      |
      | OpenQASM 3.0 emitter (Rust, walks MLIR IR via C API)
      v
[ OpenQASM 3.0 ]
```

### 6.2 `quantum.circ` Dialect

#### Invariants enforced by the verifier

1. All ops are unitary — no `quantum.dynamic` or measurement ops may appear.
2. Every `!qubit` SSA value has exactly one use (linearity).
3. Every op has `clifford : BoolAttr` and `depth_contribution : IntAttr`.
4. All ops are contained in `quantum.circ.func` regions; no external qubit capture.

#### C API op registration (abbreviated)

Ops are registered programmatically in Rust at dialect initialization time. Each op provides a name, an operand/result type list, a set of named attributes, and an optional Rust verifier callback. The snippet below illustrates the pattern for `gate`:

```rust
// Registered via mlirDialectHandleLoadDialect / custom dialect init callback
fn register_gate_op(ctx: MlirContext) {
    let info = MlirOperationStateInfo {
        name: "quantum.circ.gate",
        // operands: variadic !qubit; attributes: gate_name StrAttr,
        //           clifford BoolAttr, depth_contribution I64Attr
        // results:  variadic !qubit
        verifier: Some(verify_gate_op),
    };
    mlirRegisterOperationInfo(ctx, info);
}

// compose: CircuitType × CircuitType -> CircuitType
// tensor:  CircuitType × CircuitType -> CircuitType
// adjoint: CircuitType -> CircuitType
// controlled: CircuitType × !qubit -> CircuitType
// borrow: (region: SizedRegion<1>) -> variadic !qubit
```

All attribute names, type constraints, and depth arithmetic rules (depth = lhs.depth + rhs.depth for compose, max for tensor, etc.) are enforced in the Rust verifier callbacks registered with `mlirDialectHandleLoadDialect`.

#### Key operations summary

| Operation | Signature | Depth rule |
|---|---|---|
| `quantum.circ.gate` | `name, [!qubit] -> [!qubit]` | `depth_contribution` attribute |
| `quantum.circ.compose` | `Circuit, Circuit -> Circuit` | d₁ + d₂ |
| `quantum.circ.tensor` | `Circuit, Circuit -> Circuit` | max(d₁, d₂) |
| `quantum.circ.adjoint` | `Circuit -> Circuit` | preserved |
| `quantum.circ.controlled` | `Circuit, !qubit -> Circuit` | d + 1 |
| `quantum.circ.borrow` | `Region -> [!qubit]` | body depth |

### 6.3 `quantum.dynamic` Dialect

#### Invariants enforced by the verifier

1. Measurement ops consume `!qubit` (linear) and produce `!bit` (unrestricted).
2. Feed-forward is modeled as `cf.cond_br` on `!bit` values — standard MLIR control flow.
3. Unitary sub-circuits are embedded in `quantum.dynamic.unitary_region` blocks containing only `quantum.circ` ops.
4. Every `!qubit` in scope is eventually consumed by a gate, measurement, or `borrow` terminator.

#### Key operations summary

| Operation | Signature | Notes |
|---|---|---|
| `quantum.dynamic.measure` | `!qubit -> !bit` | Consumes qubit; produces classical bit |
| `quantum.dynamic.reset` | `!qubit -> !qubit` | Measure-and-reprepare to \|0⟩ |
| `quantum.dynamic.unitary_region` | `Region [quantum.circ ops]` | Embedded unitary block |
| `quantum.dynamic.if` | `!bit, Region, Region` | Classical branch on measurement |
| `quantum.dynamic.barrier` | `[!qubit]` | Synchronization point |

### 6.4 `quantum.physical` Dialect

Physical hardware information is represented as op attributes on `quantum.dynamic` ops, not as a separate dialect. This avoids an additional conversion pass while keeping hardware mapping explicit and verifier-checkable.

#### Physical attributes

| Attribute | MLIR type | Description |
|---|---|---|
| `phys_qubit` | `I32Attr` | Physical qubit index on the target device |
| `native_gate` | `BoolAttr` | Whether this op is in the target's native gate set |
| `fidelity` | `F64Attr` | Per-op fidelity from the backend noise model |

The physical lowering pass assigns `phys_qubit` values via the routing algorithm (§7.3) and sets `native_gate` after decomposition (§7.4). The `fidelity` attribute is populated from the backend noise model and read by the SABRE cost function.

---

## 7. Optimization Passes

All passes are implemented in Rust and registered with the MLIR pass manager as external passes via `mlirRegisterExternalPass`. Each pass provides a Rust callback struct conforming to `MlirExternalPassCallbacks` (construct, destruct, initialize, clone, run). The run callback receives an `MlirOperation` (the module root) and rewrites it in-place using `mlirOperationWalk`, `mlirOperationReplace`, and `mlirOperationErase`. Passes are composed via `mlirPassManagerAddOwnedPass`.

### 7.1 Pass Pipeline

```
quantum.circ passes:
  1. gate-cancellation
  2. rotation-merging
  3. compiler-uncomputation
  4. zx-simplification
  5. clifford-t-optimization

quantum.dynamic passes:
  6. measurement-deferral
  7. classical-region-fusion

quantum.physical passes (after physical lowering):
  8. depth-aware-sabre-routing
  9. native-gate-decomposition
  10. depth-optimal-scheduling
```

Passes 1–5 run to fixpoint before lowering to `quantum.dynamic`. Passes 8–10 run in strict order.

### 7.2 `quantum.circ` Passes

#### Gate Cancellation

Peephole pass over the op list of each `quantum.circ.func` region. Maintains a per-qubit sliding window of recent ops. Identifies adjacent self-inverse pairs and removes both:

```
H · H = I      X · X = I      Y · Y = I      Z · Z = I
S · S† = I     T · T† = I     CNOT · CNOT = I   CZ · CZ = I
SWAP · SWAP = I
```

Implementation: the Rust pass walks each `quantum.circ.func` region with `mlirBlockGetFirstOperation` / `mlirOperationGetNextInBlock`, maintains a per-qubit sliding window, and erases matched pairs with `mlirOperationErase`. No `RewritePatternSet` is required — the cancellation patterns are simple enough to express as direct IR mutations.

#### Rotation Merging

Scans each qubit's instruction stream. Consecutive same-axis rotations are merged:

```
Rz(θ₁) · Rz(θ₂)  →  Rz(θ₁ + θ₂)
Rx(θ₁) · Rx(θ₂)  →  Rx(θ₁ + θ₂)
Ry(θ₁) · Ry(θ₂)  →  Ry(θ₁ + θ₂)
```

Merged rotations where `θ_total mod 2π = 0` are eliminated entirely. Depth attribute updated after merging.

#### Compiler-Assisted Uncomputation

Scans for `quantum.circ.borrow` blocks where the region body does not contain the adjoint of its initial sub-circuit. If the entire body is a reversible circuit (all ops have inverses registered in the gate table), appends `adjoint(body)` before the block terminator. Applied before ZX simplification so inserted adjoints are eligible for further simplification.

#### ZX-Calculus Simplification

The circuit is translated into an auxiliary ZX-graph (`ZXGraph`, a Rust struct in the `zx` module). The MLIR pass (also Rust) constructs the `ZXGraph` directly — no FFI boundary is needed since both live in the same Rust binary. The translation is:

- Each qubit wire becomes an input/output boundary
- Each gate is expanded to a ZX-diagram using standard decompositions (e.g. CNOT → Z-spider connected to X-spider)
- Adjacent same-color spiders are fused

The following rewrite rules are applied to fixpoint via a worklist algorithm:

| Rule | Description |
|---|---|
| Spider fusion | Two same-color spiders connected by a single edge: merge, sum phases |
| Identity removal | Zero-phase spider with two legs: remove (wire pass-through) |
| π-copy | π-phase Z-spider through X-spider: copy and shift phases |
| Bialgebra | Z-spider fully connected to X-spider (CNOT cluster): expand and simplify |
| Euler decomposition | Three-spider Z-X-Z sequence: convert to single Rz-Rx-Rz with reduced count |
| Color change | H-box between spiders: flip color, remove H-box |
| State copy | Zero-phase spider connected to \|0⟩ boundary: remove |

After fixpoint, the ZX-graph is converted back to a gate circuit using the Euler decomposition, targeting the minimum gate count representation.

#### Clifford+T Optimization

Branches on the `clifford` attribute of the top-level circuit region:

**Clifford branch:** Builds a stabilizer tableau representation of the Clifford circuit. Propagates Pauli operators through the circuit symbolically. Identifies sequences that compose to identity and removes them. Uses the Aaronson-Gottesman tableau simulation algorithm.

**Universal branch:** Minimizes T-count using a greedy phase polynomial representation. Identifies T and T† gates that can be combined or cancelled via the phase polynomial `(-1)^(f(x))` representation. References the Todd algorithm structure. Each T-gate reduction saves one non-Clifford gate — important for fault-tolerant cost.

### 7.3 `quantum.dynamic` Passes

#### Measurement Deferral

Applies the principle of deferred measurement: a mid-circuit measurement followed by a classical conditional `if b then U else V` is equivalent (for output distributions) to a controlled operation `controlled(U†V)` followed by a terminal measurement, when no further classically conditioned operations depend on the measurement outcome before the end of the circuit.

The pass performs a reachability analysis on the `!bit` def-use chains to identify deferrable measurements and rewrites them. This reduces the number of classical/quantum context switches in the execution schedule.

#### Classical Region Fusion

Identifies adjacent `quantum.dynamic.if` blocks with:
1. Disjoint qubit operand sets (no shared `!qubit` values)
2. No data dependency between their `!bit` conditions

Merges them into a single `quantum.dynamic.if` block with parallel `unitary_region` sub-circuits. Reduces classical/quantum boundary crossings.

### 7.4 `quantum.physical` Passes

#### Depth-Aware SABRE Routing

Maps logical qubit SSA values to physical qubit indices on the device topology graph and inserts `quantum.circ.gate("SWAP", ...)` ops to satisfy connectivity constraints.

**Standard SABRE cost function:**
```
cost_basic(M) = (1 / |F|) Σ_{(u,v) ∈ F} dist(M(u), M(v))
```

Where `F` is the front layer of the circuit DAG (gates with no unexecuted predecessors) and `dist` is shortest-path distance in the connectivity graph.

**Quon extended cost function:**
```
cost(M) =
    α · swap_count(M, F)
  + β · critical_path_delta(M, W)
  + γ · noise_weight(M, noise_model)

critical_path_delta(M, W) =
    depth(schedule_ASAP(apply_mapping(M, W))) - depth(schedule_ASAP(W))

noise_weight(M, N) =
    Σ_{(u,v) ∈ F} -log(fidelity(N, M(u), M(v)))
```

Where `W` is the lookahead window (next `w` layers beyond `F`, default `w = 20`) and `N` is the backend noise model.

Parameters `α`, `β`, `γ` are configurable per-compilation. Default: `α = 1.0`, `β = 0.5`, `γ = 0.3`.

`critical_path_delta` directly operationalizes the depth type from `quantum.circ`: the type-level depth bound becomes a concrete optimization objective. Depth-increasing SWAP insertions are penalized proportional to their critical path impact, not just their gate count.

#### Native Gate Decomposition

Decomposes all `quantum.circ.gate` ops not in the backend's native gate set into sequences of native gates.

**Single-qubit decomposition:** ZYZ Euler decomposition. Any U ∈ SU(2) decomposes as:
```
U = Rz(α) · Ry(β) · Rz(γ)
```
for some α, β, γ ∈ [0, 2π). Produces at most 3 native single-qubit gates. If β = 0 (diagonal matrix), produces at most 1 Rz gate.

**Two-qubit decomposition:** Cartan (KAK) decomposition. Any U ∈ SU(4) decomposes as:
```
U = (A₁ ⊗ A₂) · exp(i(c₁ X⊗X + c₂ Y⊗Y + c₃ Z⊗Z)) · (B₁ ⊗ B₂)
```
where A₁, A₂, B₁, B₂ ∈ SU(2) and c₁, c₂, c₃ ∈ ℝ. This produces at most 3 CNOT gates (or 2 if the unitary is equivalent to a partial entangler, 0 if separable).

The native gate set is read from the `BackendTarget` descriptor; no gate set is hardcoded.

#### Depth-Optimal Scheduling

Given a physically routed circuit with `phys_qubit` attributes assigned, schedules gate ops to minimize total circuit depth.

**ASAP scheduling:** Each gate is scheduled at the earliest time step at which all its operands are available (all predecessor gates have completed). Minimizes total depth; reduces decoherence exposure.

**ALAP scheduling:** Each gate is scheduled as late as possible while respecting the circuit's output deadline. Reduces idle qubit time, beneficial for hardware where idle qubits accumulate T1 decay faster than active qubits.

**Selection rule:** If `T1 < circuit_depth * gate_time` for any qubit (i.e. the circuit depth exceeds one decoherence time), ALAP is preferred. Otherwise ASAP. This threshold is read from the noise model in the `BackendTarget` descriptor.

`quantum.dynamic.barrier` ops are respected: no gate may be scheduled across a barrier.

---

## 8. Backend Architecture

### 8.1 Backend Target Descriptor

```rust
pub struct NativeGate {
    pub name: String,
    pub num_qubits: usize,
    pub decompose: Box<dyn Fn(&[f64]) -> Vec<GateOp> + Send + Sync>,
}

pub struct NoiseModel {
    pub single_qubit_fidelity: HashMap<(String, usize), f64>,
    pub two_qubit_fidelity:    HashMap<(String, usize, usize), f64>,
    pub t1_us:       HashMap<usize, f64>,
    pub t2_us:       HashMap<usize, f64>,
    pub readout_error: HashMap<usize, f64>,
}

pub struct ConnectivityGraph {
    pub num_qubits: usize,
    pub edges: Vec<(usize, usize)>,
    pub dist: Vec<Vec<usize>>,  // Floyd-Warshall, precomputed at construction
}

pub struct BackendTarget {
    pub id: String,
    pub num_qubits: usize,
    pub topology: ConnectivityGraph,
    pub native_gates: Vec<NativeGate>,
    pub noise: NoiseModel,
    pub meas_latency_us: f64,
    pub supports_mid_circuit_meas: bool,
    pub supports_feed_forward: bool,
}
```

### 8.2 Provided Targets

| Target ID | Connectivity | Notes |
|---|---|---|
| `generic_openqasm` | All-to-all (abstract) | No noise model; used for emission testing and correctness verification |
| User-supplied | Any | Loaded from a JSON descriptor file passed via `--target target.json` |

The `generic_openqasm` target accepts all gates in the OpenQASM 3.0 standard gate library without decomposition. Routing is a no-op (all-to-all). Scheduling is ASAP.

### 8.3 Target Descriptor JSON Format

Users may supply custom targets as JSON:

```json
{
  "id": "my_device",
  "num_qubits": 5,
  "topology": {
    "edges": [[0,1],[1,2],[2,3],[3,4],[0,2]]
  },
  "native_gates": ["cx", "rz", "sx", "x"],
  "noise": {
    "single_qubit_fidelity": {"rz": {"0": 0.999, "1": 0.998}},
    "two_qubit_fidelity": {"cx": {"0,1": 0.995, "1,2": 0.992}},
    "t1_us": {"0": 120.0, "1": 115.0},
    "t2_us": {"0": 80.0,  "1": 75.0},
    "readout_error": {"0": 0.01, "1": 0.015}
  },
  "meas_latency_us": 0.9,
  "supports_mid_circuit_meas": true,
  "supports_feed_forward": true
}
```

---

## 9. Emission and Runtime Integration

### 9.1 OpenQASM 3.0 Emitter

The emitter performs a linear traversal of the `quantum.physical` IR and generates OpenQASM 3.0 source text.

#### IR → OpenQASM mapping

| IR construct | OpenQASM 3.0 output |
|---|---|
| `qreg(n)` allocation | `qubit[n] q;` |
| `quantum.dynamic.measure` on `phys_qubit=i` | `measure q[i] -> c[i];` |
| `quantum.dynamic.reset` | `reset q[i];` |
| `quantum.dynamic.if` on `!bit b` | `if (c[b_idx] == 1) { ... }` |
| `quantum.dynamic.barrier` | `barrier q[i], q[j], ...;` |
| `quantum.circ.gate` with `native_gate=true` | `gate_name q[phys_qubit], ...;` |
| `quantum.circ.gate` with `native_gate=false` | Error — decomposition pass must have run |

#### Classical bit register

The emitter maintains a classical bit register `c` parallel to the qubit register `q`. Each `measure` op allocates the next available `c[i]` slot. `if` conditions reference `c[i]` by the index of the producing measurement.

#### Gate name mapping

Gate names in `quantum.circ.gate` ops are mapped to OpenQASM 3.0 standard gate names via a lookup table. User-defined gates are emitted as `gate` declarations at the top of the output file if they are not reducible to standard gates after optimization.

### 9.2 Qiskit Aer Integration

`quon_aer.py` is a thin Python verification bridge:

```python
import subprocess, sys
from qiskit import qasm3
from qiskit_aer import AerSimulator

def run(source_file: str, shots: int = 4096) -> dict:
    result = subprocess.run(
        ["quonc", "--emit-qasm", source_file],
        capture_output=True, text=True, check=True
    )
    qasm_src = result.stdout
    circuit   = qasm3.loads(qasm_src)
    circuit.measure_all()
    sim       = AerSimulator()
    job       = sim.run(circuit, shots=shots)
    counts    = job.result().get_counts()
    return counts
```

Usage:

```bash
quonc --emit-qasm program.qn | python quon_aer.py --shots 8192
```

**Verification workflow:**
1. Implement a known algorithm in Quon (Bell state, GHZ, Grover's, QFT, Shor's for small N)
2. Compile with `quonc`
3. Run `quon_aer.py`
4. Compare output histogram against the known theoretical distribution

---

## 10. Implementation Plan

### 10.1 Repository Structure

Pure Rust workspace. No CMakeLists.txt, no TableGen, no C++ sources.

```
quon/
├── Cargo.toml                        # Workspace root; members = [quonc, frontend, mlir_bridge, backend, zx]
│
├── quonc/                            # Binary crate — compiler driver
│   ├── Cargo.toml
│   └── src/
│       └── main.rs                   # CLI entry point: arg parsing, pipeline orchestration
│
├── frontend/                         # Library crate — language frontend
│   ├── Cargo.toml
│   └── src/
│       ├── lexer.rs                  # Tokenizer
│       ├── parser.rs                 # Recursive-descent parser → AST
│       ├── ast.rs                    # AST type definitions
│       ├── types.rs                  # Type definitions, kind checker
│       ├── typecheck.rs              # Bidirectional type checker, linear context
│       ├── refinement.rs             # Symbolic depth arithmetic, Z3 bridge
│       └── lower.rs                  # AST → in-memory MLIR (calls mlir_bridge)
│
├── zx/                               # Library crate — ZX-graph
│   ├── Cargo.toml
│   └── src/
│       ├── graph.rs                  # ZXGraph data structure
│       └── rewrite.rs                # Rewrite rules (fixpoint worklist)
│
├── mlir_bridge/                      # Library crate — MLIR/LLVM C API bindings + dialect + passes
│   ├── Cargo.toml
│   ├── build.rs                      # Links libMLIR + libLLVM; emits cargo:rustc-link-lib directives
│   └── src/
│       ├── sys.rs                    # Raw unsafe bindings to MLIR/LLVM C API (mlir-sys style)
│       ├── context.rs                # MlirContext wrapper
│       ├── dialect/
│       │   ├── mod.rs
│       │   ├── quantum_circ.rs       # quantum.circ dialect registration (C API)
│       │   ├── quantum_dynamic.rs    # quantum.dynamic dialect registration
│       │   └── quantum_physical.rs   # Physical attribute helpers
│       ├── passes/
│       │   ├── mod.rs                # Pass registration helpers
│       │   ├── gate_cancellation.rs
│       │   ├── rotation_merging.rs
│       │   ├── compiler_uncomputation.rs
│       │   ├── zx_simplification.rs  # Calls into zx crate directly (no FFI)
│       │   ├── clifford_t_opt.rs
│       │   ├── measurement_deferral.rs
│       │   ├── classical_region_fusion.rs
│       │   ├── sabre_routing.rs
│       │   ├── native_gate_decomp.rs
│       │   └── depth_scheduling.rs
│       └── emit/
│           └── openqasm3.rs          # IR → OpenQASM 3.0 (walks via C API)
│
├── backend/                          # Library crate — target descriptors
│   ├── Cargo.toml
│   └── src/
│       ├── target.rs                 # BackendTarget, NoiseModel, ConnectivityGraph
│       ├── json.rs                   # JSON loader (serde_json)
│       └── generic_openqasm.rs       # generic_openqasm built-in target
│
├── python/
│   └── quon_aer.py                   # Qiskit Aer verification bridge
│
└── test/
    ├── lit.cfg.py                    # LLVM lit configuration (uses quonc binary)
    ├── lit/
    │   ├── circ/                     # quantum.circ round-trip and pass tests
    │   ├── dynamic/                  # quantum.dynamic tests
    │   ├── physical/                 # Routing and scheduling tests
    │   └── emit/                     # OpenQASM emission FileCheck tests
    └── verify/
        ├── bell.qn
        ├── grover.qn
        ├── qft.qn
        └── ising.qn
```

### 10.2 Build System

**Everything is Cargo.** No CMake, no separate build step for C++ or TableGen.

```bash
# Build the full compiler
cargo build --release

# Run the compiler
./target/release/quonc program.qn --target generic_openqasm --emit-qasm
```

`mlir_bridge/build.rs` delegates LLVM/MLIR library discovery to Melior's build script, which emits the appropriate `cargo:rustc-link-lib` and `cargo:rustc-link-search` directives. If LLVM 22 is not on the default search path, set `MLIR_SYS_220_PREFIX` to the LLVM/MLIR install prefix (e.g. `/usr/lib/llvm-22` on Linux, `$(brew --prefix llvm@22)` on macOS).

**Prerequisites:**
- **LLVM 22 + MLIR** — built from the monorepo or installed via apt.llvm.org (`./llvm.sh 22`) or Homebrew (`brew install llvm@22`), with the C API enabled (`-DLLVM_ENABLE_PROJECTS=mlir`, `-DMLIR_ENABLE_BINDINGS_PYTHON=OFF`)
- **Melior 0.27.x** — pinned in the workspace `Cargo.toml`; pulls in `mlir-sys` 220.x
- **`libz3`** (C API) for the refinement checker in `frontend`
- Rust stable toolchain (edition 2021)

**No C++ compiler required at build time.**

### 10.3 Milestones

| Phase | Deliverables | Key Skill Demonstrated |
|---|---|---|
| 1 — Dialect Foundation | `quantum.circ` and `quantum.dynamic` dialect registration via MLIR C API in Rust (`mlir_bridge`), op verifier callbacks, round-trip FileCheck tests | MLIR C API, dialect design, Rust FFI patterns |
| 2 — Frontend | Rust lexer, parser, bidirectional type checker with split linear context, Circuit type with symbolic depth, Z3 refinement bridge | Linear type theory, refinement types, Rust |
| 3 — Lowering | AST-to-`quantum.circ` lowering (building MLIR in-memory via C API), monadic lowering external pass, conversion pass infrastructure | MLIR C API op construction, progressive lowering |
| 4 — `quantum.circ` Passes | Gate cancellation, rotation merging, ZX-calculus rewriting (Rust `zx` crate, no FFI boundary), Clifford+T optimization, uncomputation; all as `mlirRegisterExternalPass` passes | Algebraic circuit optimization, graph rewriting, ZX-calculus, MLIR pass infrastructure |
| 5 — Physical Layer | Rust `backend` crate with `BackendTarget`/`NoiseModel`, JSON loader (`serde_json`), depth-aware SABRE routing, native gate decomposition (KAK), noise-weighted scheduling | Hardware-aware compilation, routing algorithms, decomposition theory |
| 6 — Emission + Verification | OpenQASM 3.0 emitter (walks IR via `mlirOperationWalk`), `quon_aer.py` Qiskit bridge, end-to-end test suite across all reference algorithms | End-to-end integration, simulation verification |

---

## 11. Key Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Input language paradigm | Functional with linear types | Encodes no-cloning at the type level; circuit composition as function composition exposes structure for ZX rewriting and Clifford analysis |
| Surface syntax | Kotlin-style `fn` definitions, inline annotations, `circuit { }` / `run { }` blocks, `\|>` composition, `@` application | Familiar to engineers; block forms make unitary/dynamic boundary syntactically explicit |
| Feed-forward | Explicit `run { }` with monadic bind | Makes classical/quantum data dependency visible in the type; maps directly to `quantum.dynamic` SSA structure |
| Symbolic depth | Linear arithmetic over runtime variables, Z3 for constraint checking | Eliminates need for `DynCircuit` in variational algorithms; depth remains informative even when parameterized at runtime |
| Type-level Clifford | Inferred from gate primitives, propagated by composition | Never user-annotated; directly restricts and motivates Clifford+T and ZX optimization passes |
| Register destructuring | Explicit `destructure` / `split` only — no indexing | Keeps linear context simple: `QReg<n>` is one linear value; per-element borrow tracking is avoided |
| Ancilla | `borrow` blocks + compiler-assisted uncomputation | Static safety guarantee + automatic optimization of uncomputation patterns |
| Dialect separation | Strict `quantum.circ` (unitary) / `quantum.dynamic` (dynamic) boundary | Preserves algebraic invariants required for ZX and Clifford passes; prevents incorrect rewrites on dynamic circuits |
| Physical qubit assignment | Attributes on `quantum.dynamic` ops, not a fourth dialect | Avoids additional conversion pass; physical info is metadata, not a different computation model |
| ZX-calculus placement | Rewriting pass on `quantum.circ` | Only valid at the unitary level; enables non-local identities missed by peephole passes |
| SABRE extension | Depth + noise terms in cost function | Operationalizes the type-level depth bound at the physical layer; genuine algorithmic contribution beyond standard SABRE |
| Hardware targeting | Abstract pluggable `BackendTarget` descriptor; no vendor hardcoded | Keeps compiler general; OpenQASM 3.0 is the universal output target |
| Implementation split | Entirely Rust; LLVM and MLIR accessed via their stable C APIs | Single language across the whole compiler; MLIR's C API (`mlirRegisterExternalPass`, `mlirOperationWalk`, `mlirPassManagerRun`) is sufficient for all dialect registration, pass authoring, and IR mutation without requiring C++ or TableGen |
| ZX-graph location | Rust `zx` crate, imported directly by the `zx_simplification` pass | Graph algorithms are more natural in Rust; no FFI boundary needed since both the pass and the graph live in the same binary |

---

## 12. Reference Algorithms

These programs constitute the canonical test suite. Each exercises a distinct subset of language features.

### Bell State

Tests: `circuit { }` block, `run { }` block, `destructure`, basic measurement.

```kotlin
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}

fn hello_bell(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0       <- measure(q0)
    b1       <- measure(q1)
    return (b0, b1)
    -- Expected: (0,0) and (1,1) each with probability 0.5
}
```

### Quantum Teleportation

Tests: feed-forward, classical conditionals returning circuit values, tuple destructuring in `<-` bindings.

```kotlin
fn teleport(msg: Qubit, alice: Qubit, bob: Qubit): Q<Qubit> = run {
    (a, b)   <- bell_state() @ (alice, bob)
    (m2, a2) <- adjoint(bell_state()) @ (msg, a)
    x_bit    <- measure(m2)
    z_bit    <- measure(a2)
    let b2    = (if x_bit then X else identity(1)) @ b
    let b3    = (if z_bit then Z else identity(1)) @ b2
    return b3
}
```

### Grover's Search

Tests: higher-order oracle type, `repeat`, depth arithmetic from `n`.

```kotlin
type Oracle<n> = Circuit<n, n, _, Universal>

fn hadamard_all(n: Nat): Circuit<n, n, 1, Clifford> = circuit {
    for q in qubits(n) { H q }
}

fn diffusion(n: Nat): Circuit<n, n, 3, Clifford> = circuit {
    hadamard_all(n) |> phase_flip_zero(n) |> hadamard_all(n)
}

fn grover(n: Nat, oracle: Oracle<n>): Q<List<Bit>> = run {
    let iters  = round(PI / 4.0 * sqrt(2.0 ^ n))
    let search = hadamard_all(n) |> repeat(iters, oracle |> diffusion(n))
    q         <- search @ qreg(n)
    measure_all(q)
}
```

### Shor's Algorithm (Quantum Kernel)

Tests: recursive circuit construction, `adjoint`, `split`, `on_high`, depth `O(n³)`.

```kotlin
fn qft(n: Nat): Circuit<n, n, n*n, Universal> = circuit {
    match n {
        0 => identity(0),
        _ => (H @0 |> controlled_rotations(n))
             |> (qft(n-1) `on_high` n)
             |> swap_reverse(n)
    }
}

fn mod_exp(n: Nat, a: Int, nn: Int): Circuit<2*n, 2*n, n^3, Universal> = circuit {
    build_controlled_modexp(n, a, nn)
}

-- Classical post-processing (continued fractions, GCD) lives in host code.
fn shor_quantum(n: Nat, a: Int, nn: Int): Q<List<Bit>> = run {
    ctrl          <- hadamard_all(n) @ qreg(n)
    tgt           <- init_one() @ qreg(n)
    both          <- mod_exp(n, a, nn) @ (ctrl `tensored` tgt)
    let (ctrl2, _) = split(n, both)
    est           <- adjoint(qft(n)) @ ctrl2
    measure_all(est)
}
```

### 3-Qubit Bit-Flip Error Correction

Tests: `borrow`, syndrome measurement, `match` on tuples, `adjoint` for decoding.

```kotlin
fn encode(): Circuit<1, 3, 2, Clifford> = circuit {
    CNOT @(0,1) |> CNOT @(0,2)
}

fn syndrome_measure(q: QReg<3>): Q<(QReg<3>, Bit, Bit)> = run {
    let (q0, q1, q2) = destructure(q)
    borrow a1: Qubit, a2: Qubit in {
        (q0a, a1b) <- (CNOT @(0,0) |> CNOT @(1,0)) @ (q0 `tensored` a1)
        (q1a, a2b) <- (CNOT @(1,0) |> CNOT @(2,0)) @ (q1 `tensored` a2)
        s1         <- measure(a1b)
        s2         <- measure(a2b)
        return (q0a `tensored` q1a `tensored` q2, s1, s2)
    }
}

fn correct(q: QReg<3>, s1: Bit, s2: Bit): Q<QReg<3>> = run {
    return match (s1, s2) {
        (0, 0) => q,
        (1, 0) => X @0 @ q,
        (1, 1) => X @1 @ q,
        (0, 1) => X @2 @ q,
    }
}

fn bit_flip_round(logical: Qubit): Q<Qubit> = run {
    encoded            <- encode() @ logical
    (data, s1, s2)    <- syndrome_measure(encoded)
    corrected         <- correct(data, s1, s2)
    decoded           <- adjoint(encode()) @ corrected
    let (out, _rest)   = split(1, decoded)
    return out
}
```

### QAOA for QUBO

Tests: symbolic runtime depth (`n_steps * n`), `fold`, `DynCircuit`-free variational circuit.

```kotlin
fn cost_layer(n: Nat, gamma: Float, q: Matrix<n, n, Float>): Circuit<n, n, n*n, Universal> = circuit {
    for (i, j) in pairs(n) { Rzz(gamma * q[i][j]) @(i,j) }
    |> for i in diag(n)    { Rz(gamma * q[i][i]) @i }
}

fn mixer_layer(n: Nat, beta: Float): Circuit<n, n, 1, Universal> = circuit {
    for q in qubits(n) { Rx(beta) q }
}

fn qaoa_layer(n: Nat, gamma: Float, beta: Float, q: Matrix<n, n, Float>)
    : Circuit<n, n, n*n + 1, Universal> = circuit {
    cost_layer(n, gamma, q) |> mixer_layer(n, beta)
}

fn qaoa_circuit(n: Nat, p: Int, params: List<(Float, Float)>, q: Matrix<n, n, Float>)
    : Circuit<n, n, p * (n*n + 1), Universal> =
    fold(params.take(p), identity(n), fn(acc, (gamma, beta)) ->
        acc |> qaoa_layer(n, gamma, beta, q)
    )

fn qaoa_shot(n: Nat, p: Int, params: List<(Float, Float)>, q: Matrix<n, n, Float>)
    : Q<List<Bit>> = run {
    let circ = hadamard_all(n) |> qaoa_circuit(n, p, params, q)
    reg     <- circ @ qreg(n)
    measure_all(reg)
}
```

### Bernstein-Vazirani

Tests: `Oracle` type alias, `split` with `_` discard, single-shot exact recovery.

```kotlin
type BVOracle<n> = Circuit<n+1, n+1, _, Universal>

fn bernstein_vazirani(n: Nat, oracle: BVOracle<n>): Q<List<Bit>> = run {
    q              <- hadamard_all(n) @ qreg(n)
    anc            <- (H |> X) @ qreg(1)
    both           <- oracle @ (q `tensored` anc)
    let (q2, _anc)  = split(n, both)
    out            <- hadamard_all(n) @ q2
    measure_all(out)
    -- Single shot recovers the full secret string s exactly
}
```

### 1D Transverse-Field Ising Model Simulation

Tests: symbolic depth with two runtime variables (`n_steps * n`), `fold` over time steps, physically motivated parameterized gates, `map_q` for time series.

```kotlin
-- H = -J Σᵢ ZᵢZᵢ₊₁  -  h Σᵢ Xᵢ
-- First-order Trotterization: exp(-iHt) ≈ [exp(-iH_ZZ τ) · exp(-iH_X τ)]^n_steps
-- Trotter error: O(t²/n_steps). Increase n_steps to improve accuracy.

fn zz_layer(n: Nat, j: Float, tau: Float): Circuit<n, n, n-1, Universal> = circuit {
    for i in range(n-1) { Rzz(-2.0 * j * tau) @(i, i+1) }
}

fn x_layer(n: Nat, h: Float, tau: Float): Circuit<n, n, 1, Universal> = circuit {
    for q in qubits(n) { Rx(-2.0 * h * tau) q }
}

fn trotter_step(n: Nat, j: Float, h: Float, tau: Float): Circuit<n, n, n, Universal> = circuit {
    zz_layer(n, j, tau) |> x_layer(n, h, tau)
}

-- Total depth: n_steps * n  (symbolic; verified by Z3 at fold boundary)
fn ising_evolve(n: Nat, j: Float, h: Float, t: Float, n_steps: Int)
    : Circuit<n, n, n_steps * n, Universal> =
    let tau = t / float(n_steps)
    in fold(range(n_steps), identity(n), fn(acc, _) ->
        acc |> trotter_step(n, j, h, tau)
    )

fn simulate_ising(n: Nat, j: Float, h: Float, t: Float, n_steps: Int): Q<List<Bit>> = run {
    q <- ising_evolve(n, j, h, t, n_steps) @ qreg(n)
    measure_all(q)
}

-- Time series: simulate at t ∈ {0, dt, 2dt, ..., steps*dt}
-- map_q is monadic map: (A -> Q<B>, List<A>) -> Q<List<B>>
fn ising_time_series(n: Nat, j: Float, h: Float, dt: Float, steps: Int, n_trott: Int)
    : Q<List<List<Bit>>> =
    map_q(fn(k) -> simulate_ising(n, j, h, float(k) * dt, n_trott), range(steps + 1))
```

---

## 13. Reference Literature

- Lattner et al., "MLIR: Scaling Compiler Infrastructure for Domain Specific Computation" (CGO 2021)
- Li et al., "Tackling the Qubit Mapping Problem for NISQ-Era Quantum Devices" (ASPLOS 2019) — SABRE routing algorithm
- van de Wetering, "ZX-calculus for the working quantum computer scientist" (2020) — ZX rewriting rules and completeness
- Amy et al., "Polynomial-Time T-Depth Optimization of Clifford+T Circuits Via Matroid Partitioning" (IEEE TC 2014) — T-count minimization
- Ross & Selinger, "Optimal ancilla-free Clifford+T approximation of z-rotations" (2016)
- Rios & Selinger, "A categorical model for a quantum circuit description language" — Proto-Quipper foundations
- Gottesman, "The Heisenberg Representation of Quantum Computers" (1998) — Stabilizer tableau / Clifford simulation
- Aaronson & Gottesman, "Improved simulation of stabilizer circuits" (PRA 2004) — Tableau algorithm
- Cowtan et al., "On the Qubit Routing Problem" (TQC 2019) — Routing survey
- Khaneja & Glaser, "Cartan decomposition of SU(2^n)" (2001) — KAK two-qubit decomposition
- Zhang et al., "Geometric theory of nonlocal two-qubit operations" (PRA 2003) — Cartan decomposition for quantum gates
- Feynman, "Simulating Physics with Computers" (1982) — Original motivation for quantum simulation
- OpenQASM 3.0 Language Specification — https://openqasm.com
