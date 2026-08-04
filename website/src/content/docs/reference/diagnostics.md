---
title: Diagnostic catalog
description: A task-oriented catalog of Quon compiler diagnostics — start from an error, reach a minimal reproducer, root cause, and smallest supported repair.
sidebar:
  order: 1
---

When the Quon compiler rejects a program, it emits a diagnostic with a **stable
code** (e.g. `quon.linearity.used-twice`) and a span-accurate caret. This
catalog is organized by the task you were trying to accomplish, not by the
compiler module that produced the error. Each entry gives a minimal failing
program, the representative diagnostic, an explanation, and the smallest
supported repair — or a stated limitation when no automated rewrite exists.

The snippets below are drawn from or verified against the frontend integration
test suite (`frontend/tests/typecheck.rs`, `linearity.rs`, `circuits.rs`,
`lsp_diagnostics.rs`), which asserts on the exact diagnostic messages in CI.

## How to read an entry

| Field | Meaning |
|-------|---------|
| **Code** | Stable LSP diagnostic slug. Searchable in editor squiggles and `--dump-ir` output. |
| **Failing program** | The shortest `.qn` source (or CLI command) that triggers the diagnostic. |
| **Diagnostic** | The message the compiler prints, including the span-anchored caret. |
| **Explanation** | Why the checker rejected the program — the type-system rule or invariant. |
| **Repair** | The smallest source change that makes the program accepted. Quick fixes offered by `quon_lsp` are marked **[auto-fix]**. |

---

## Parse and lex errors

Errors from the lexer and parser surface before type-checking begins. They
share the `quon.lex.*`, `quon.parse.*`, and `quon.desugar.*` code prefixes.

### Invalid character

**Code:** `quon.lex.invalid-char`

**Failing program:**

```kotlin
a # b
```

**Diagnostic:**

```text
unexpected character `#`
```

**Explanation:** The lexer encountered a character that is not part of any
Quon token. `#` is not an operator, delimiter, or comment opener.

**Repair:** Remove the character or replace it with valid syntax. Block
comments use `{- ... -}` and line comments are not yet supported (see
[Limitations](#limitations) below).

---

### Unterminated block comment

**Code:** `quon.lex.unterminated-comment`

**Failing program:**

```kotlin
{- unclosed block
```

**Diagnostic:**

```text
unclosed block comment
```

**Explanation:** A `{-` opened a block comment but no matching `-}` was found
before end-of-file. The span covers everything from the opening `{-` to the
end of the source.

**Repair:** Close the comment with `-}`.

---

### Unexpected token (parse)

**Code:** `quon.parse.unexpected-token`

**Failing program:**

```kotlin
fn f(): Int = )
```

**Diagnostic:**

```text
unexpected token `)`
```

**Explanation:** The parser could not continue from this token. This is the
default bucket for all parse failures — the parser does not yet produce
per-rule error codes.

**Repair:** Fix the syntax preceding the token. Common causes: missing `=`
in a binding, missing `in` after a `let`, or a stray delimiter.

---

### Run-block trailing bind

**Code:** `quon.desugar.run-trailing-bind`

**Failing program:**

```kotlin
fn f(): Q<Int> = run { x <- measure(qubit()) }
```

**Diagnostic:**

```text
a `run` block must end in an expression, not a `<-` bind
```

**Explanation:** A `run` block's final statement must be a return expression.
A `<-` monadic bind consumes a resource but does not produce the block's
result.

**Repair:** Add a return expression after the bind:

```kotlin
fn f(): Q<Int> = run {
    x <- measure(qubit())
    x
}
```

---

## Name and builtin lookup

### Unbound variable

**Code:** `quon.type.unbound-variable`

**Failing program:**

```kotlin
fn f(): Int = ghost
```

**Diagnostic:**

```text
unbound variable `ghost`
```

**Explanation:** The name `ghost` is not bound in the classical context Γ,
the prelude, or the constants table. The caret lands on the identifier.

**Repair:** Bind the name (parameter, `let`, or `fn` parameter) or correct
the spelling. Built-in names like `H`, `CNOT`, `measure`, `identity`,
`fold`, `map`, `range`, and `qubits` are in the prelude.

---

### Not a function

**Code:** `quon.type.not-a-function`

**Failing program:**

```kotlin
fn f(): Int = 42(true)
```

**Diagnostic:**

```text
cannot apply a value of non-function type `Int`
```

**Explanation:** The callee of an application had a non-function type. `42`
is `Int`, not `A -> B`, so it cannot be called.

**Repair:** Ensure the callee is a function. If you meant to index or apply a
gate, use the correct syntax (`@` for gates, `[...]` for indexing).

---

## Classical type mismatches

### Type mismatch

**Code:** `quon.type.mismatch`

**Failing program:**

```kotlin
fn f(): Int = false
```

**Diagnostic:**

```text
type mismatch: expected `Int`, found `Bool`
```

**Explanation:** The body's inferred type does not match the declared return
type. This is the workhorse unification failure — it also fires for `if`
branch disagreement, argument mismatches, and ascription errors. The caret
lands on the offending sub-expression.

**Repair:** Align the body with the return type, or change the return type.

---

### Arity mismatch

**Code:** `quon.type.arity-mismatch`

**Diagnostic:**

```text
expected a 2-tuple, found 1 components
```

**Explanation:** A tuple pattern or expression had the wrong number of
components for its type. Triggered when destructuring or constructing a tuple
whose width disagrees with the annotated type.

**Repair:** Adjust the pattern or expression to match the tuple width.

---

### Non-exhaustive match

**Code:** `quon.type.non-exhaustive-match`

**Failing program:**

```kotlin
fn f(b: Bool): Int = match b { true => 1 }
```

**Diagnostic:**

```text
non-exhaustive `match`: pattern `false` not covered
```

**Explanation:** The `match` does not cover every value of the scrutinee's
type. The diagnostic names a **witness pattern** — a concrete value not
matched — so you know exactly what is missing. For tuple scrutinees, it
names the missing corner, e.g. `(false, _)`.

**Repair:** Add an arm for the witness pattern, or add a wildcard `_` arm.

---

### Unreachable arm

**Code:** `quon.type.unreachable-arm`

**Failing program:**

```kotlin
fn f(n: Int): Int = match n { _ => 0, 5 => 1 }
```

**Diagnostic:**

```text
unreachable `match` arm
```

**Explanation:** A `match` arm can never be reached because earlier arms
already cover it. The caret lands on the dead pattern.

**Repair:** Remove the unreachable arm or reorder arms so the specific
pattern precedes the wildcard.

---

### Ambiguous lambda

**Code:** `quon.type.ambiguous-lambda`

**Failing program:**

```kotlin
fn f(): Int = let g = fn(x) -> x in g(1)
```

**Diagnostic:**

```text
cannot infer the type of this lambda; add parameter annotations or an expected type
```

**Explanation:** A lambda with unannotated parameters appeared where its type
could not be inferred top-down. Without a known callee or ascription, the
checker cannot assign types to the parameters.

**Repair:** Annotate the parameter (`fn(x: Int) -> x`) or ascribe the
binding (`let g: Int -> Int = fn(x) -> x`).

---

### Infinite type (occurs check)

**Code:** `quon.type.infinite-type`

**Diagnostic:**

```text
cannot construct an infinite type
```

**Explanation:** Unification would produce an infinite type, e.g. `?0 =
List<?0>`. This indicates a cyclic type constraint that has no finite
solution.

**Repair:** Break the cycle by introducing an explicit type annotation that
prevents the recursive unification.

---

### Alias arity

**Code:** `quon.type.alias-arity`

**Diagnostic:**

```text
type alias `Oracle` expects 2 argument(s), found 1
```

**Explanation:** A type alias was referenced with the wrong number of type
arguments.

**Repair:** Supply the correct number of type arguments.

---

### Not numeric

**Code:** `quon.type.not-numeric`

**Failing program:**

```kotlin
fn f(b: Bool): Int = -b
```

**Diagnostic:**

```text
arithmetic requires `Int` or `Float`, found `Bool`
```

**Explanation:** A value was used in arithmetic (negation, `+`, `-`, `*`,
`/`, `^`) but is not `Int` or `Float`. The caret lands on the offending
operand.

**Repair:** Convert the value to a numeric type or remove the arithmetic
operation.

---

### Kind mismatch

**Code:** `quon.type.kind-mismatch`

**Diagnostic:**

```text
kind mismatch: expected `Nat`, found `CodeFamily`
```

**Explanation:** A type argument has the wrong kind. Type parameters are
either `Nat` (for width/depth/distance) or `CodeFamily` (for QEC family
tags). Supplying a `Nat` where `CodeFamily` is required (or vice versa) is a
kind error, not a type error.

**Repair:** Supply a type argument of the correct kind.

---

## Linearity

Quon's linear type system tracks quantum resources (`Qubit`, `QReg`,
`Circuit` values, `QecBlock`) in a separate linear context Δ. Every binding
in Δ must be consumed exactly once. These errors are **type errors**, not
warnings — the program does not compile until every resource is accounted
for. See the [linear type system](../language/linearity/) guide for the
full theory.

### Used twice (no-cloning)

**Code:** `quon.linearity.used-twice`

**Failing program:**

```kotlin
fn f(q: Qubit): QReg<2> = (q, q)
```

**Diagnostic:**

```text
linear resource `q` is used more than once (no-cloning)
```

**Related info:** "first use of `q`" — the caret on the diagnostic points at
the **second** use; a related span marks the first.

**Explanation:** A linear resource was used more than once. In linear logic,
contraction is absent — you cannot duplicate a qubit. This is the type-level
enforcement of the no-cloning theorem.

**Repair:** Use two distinct qubits, or consume `q` once and produce a
register via a gate: `fn f(q: Qubit): QReg<2> = q |> H |> ...`.

---

### Never consumed (no-dropping)

**Code:** `quon.linearity.unconsumed`

**Failing program:**

```kotlin
fn f(q: Qubit): Int = 0
```

**Diagnostic:**

```text
linear resource `q` is never consumed (no-dropping)
```

**Explanation:** A linear resource went out of scope without being consumed.
Weakening is absent from the linear context — you cannot silently drop a
qubit. The caret lands on the binding that introduced the resource.

**Repair:** Consume the resource before it goes out of scope — `measure(q)`,
`discard(q)`, or thread it into the return value. For borrowed ancillae,
**[auto-fix]** `quon_lsp` offers a `discard(a)` or `reset(a)` quick fix.

---

### Branch mismatch

**Code:** `quon.linearity.branch-mismatch`

**Failing program:**

```kotlin
fn f(c: Bool, q: Qubit, q2: Qubit): Qubit = if c then q else q2
```

**Diagnostic:**

```text
linear resource `q` is consumed in some branches but not all
```

**Explanation:** The branches of an `if`/`match` disagree on which linear
resources they consume. If `q` is spent on one path but not all, the
resource's fate depends on a runtime condition — which the type system cannot
permit. The caret lands on the offending branch.

**Repair:** Ensure every branch consumes the same set of linear resources.
For qubits you intend to discard conditionally, `measure` or `discard` them
in every branch.

---

### Discard with wildcard

**Code:** `quon.linearity.discard`

**Failing program:**

```kotlin
fn f(q: Qubit): Int = let _ = q in 0
```

**Diagnostic:**

```text
cannot discard linear resource `Qubit` with `_`
```

**Explanation:** A linear resource was bound to a wildcard `_`, silently
discarding it. The linear type system requires explicit consumption.

**Repair:** Replace `let _ = q` with an explicit `discard(q)` or
`measure(q)`. Inside `run` blocks, **[auto-fix]** `quon_lsp` offers a
`discard(q)` quick fix. In `let ... in` expressions outside `run`, no
automated rewrite is offered — see [Limitations](#limitations).

---

### Capture in closure

**Code:** `quon.linearity.capture`

**Failing program:**

```kotlin
fn f(q: Qubit): (Qubit) -> Qubit = fn(x) -> q
```

**Diagnostic:**

```text
cannot capture linear resource `q` in a closure
```

**Explanation:** A lambda body referred to a linear resource from the
enclosing scope. A function value may run zero or many times, so it cannot
guarantee consuming a resource exactly once.

**Repair:** Pass the resource as an explicit parameter, or restructure so the
closure does not need the captured resource. Use a linear arrow (`-o`) for
functions that consume their argument exactly once.

---

## Circuit composition

### Not a circuit

**Code:** `quon.circuit.not-a-circuit`

**Failing program:**

```kotlin
fn f(): Circuit<2, 2, 2, Clifford> = identity(2) |> 42
```

**Diagnostic:**

```text
expected a circuit, found `Int`
```

**Explanation:** A value was used as a circuit (composed, placed, adjointed,
or controlled) but is not one. `42` is `Int`, not `Circuit<...>`.

**Repair:** Replace the non-circuit value with a circuit value, or remove the
composition operator.

---

### Qubit count mismatch

**Code:** `quon.circuit.qubit-count-mismatch`

**Failing program:**

```kotlin
fn f(): Circuit<2, 3, 0, Clifford> = identity(2) |> identity(3)
```

**Diagnostic:**

```text
circuit composition requires matching qubit counts:
left produces 2, right consumes 3
```

**Explanation:** Sequential composition `f |> g` requires `f`'s output width
to equal `g`'s input width. Here `identity(2)` produces 2 qubits but
`identity(3)` consumes 3.

**Repair:** Align the widths — either change one circuit's width or insert a
width-changing operation between them.

---

### Gate target arity

**Code:** `quon.circuit.gate-target-arity`

**Failing program:**

```kotlin
fn f(): Circuit<2, 2, 1, Clifford> = circuit { CNOT @0 }
```

**Diagnostic:**

```text
this gate acts on 2 qubit(s), but 1 target(s) were given
```

**Explanation:** A gate was placed on the wrong number of qubit targets.
`CNOT` is a two-qubit gate; `@0` provides only one target.

**Repair:** Provide the correct number of targets: `CNOT @(0, 1)`.

---

### Index out of bounds

**Code:** `quon.circuit.index-out-of-bounds`

**Failing program:**

```kotlin
fn f(): Circuit<1, 1, 1, Clifford> = circuit { H @1 }
```

**Diagnostic:**

```text
qubit index 1 is out of bounds for a register of width 1
```

**Explanation:** A gate was targeted at a qubit index outside the ambient
register `0..width`. For a 1-qubit register, valid indices are `{0}`.

**Repair:** Use a valid index, or increase the register width.

---

## Refinement: Clifford and depth

### Clifford classification mismatch

**Code:** `quon.refinement.clifford-mismatch`

**Failing program:**

```kotlin
fn f(): Circuit<1, 1, 1, Clifford> = circuit { T @0 }
```

**Diagnostic:**

```text
Clifford classification mismatch: annotated `Clifford`, inferred `Universal`
```

**Explanation:** A user-supplied Clifford annotation disagrees with the
inferred classification. `T` is a non-Clifford gate, so the circuit is
`Universal`, not `Clifford`. The classification is inferred bottom-up from
the gates, not assumed from the annotation. See [Clifford
classification](../language/clifford/).

**Repair [auto-fix]:** `quon_lsp` offers a quick fix to change the annotation
to `Universal`. Alternatively, remove the non-Clifford gate or change the
annotation.

---

### Depth mismatch

**Code:** `quon.refinement.depth-mismatch`

**Failing program:**

```kotlin
fn bell(): Circuit<2, 2, 1, Clifford> = circuit { H @0 |> CNOT @(0, 1) }
```

**Diagnostic:**

```text
circuit depth mismatch: annotated `1`, inferred `(+ 1 1)`
```

**Explanation:** A user-supplied depth annotation could not be shown equal to
the inferred symbolic depth. `H` and `CNOT` are sequential (same qubit
dependency), so the depth is 2, not 1. The inferred depth is a symbolic
expression (here `(+ 1 1)`) verified by Z3 or a constant fast-path. See
[Depth bounds](../language/depth/).

**Repair [auto-fix]:** When the inferred depth is a concrete natural,
`quon_lsp` offers a quick fix to update the annotation to the inferred value
(e.g. `Circuit<2, 2, 2, Clifford>`). Otherwise, loosen the annotation or
reduce the circuit's sequential depth.

---

### Depth intractable

**Code:** `quon.refinement.depth-intractable`

**Diagnostic:**

```text
depth constraint `n*n + m` is too complex for the solver to verify;
supply a static depth bound
```

**Explanation:** A symbolic depth constraint was beyond what the refinement
solver (Z3) could decide — typically an intractable nonlinear term. The
solver timed out or returned `unknown`.

**Repair:** Supply a static depth bound (a concrete `Nat` literal or a
simpler expression the solver can discharge). There is no automated rewrite.

---

## Monad and borrow

### Expected monad

**Code:** `quon.monad.expected-monad`

**Failing program:**

```kotlin
fn f(): Q<Int> = run {
    x <- 42
    x
}
```

**Diagnostic:**

```text
the right-hand side of `<-` must be a quantum computation `Q<_>`, found `Int`
```

**Explanation:** The right-hand side of a monadic `<-` bind was expected to be
a quantum computation `Q<_>` (or a pure value, auto-lifted) but was something
else. `42` is `Int`, not `Q<Int>`. See [The Quantum Monad](../language/monad/).

**Repair:** Wrap the value in a quantum computation, or use a `let` binding
instead of `<-` for pure values.

---

### Borrow escape

**Code:** `quon.borrow.escape`

**Failing program:**

```kotlin
fn f(): Q<Qubit> = run {
    borrow a: Qubit in {
        return a
    }
}
```

**Diagnostic:**

```text
borrowed ancilla `a` escapes its borrow scope; it must be measured,
`reset`, or `discard`ed inside the block, not returned
```

**Related info:** "borrowed as `a` here" — marks the `borrow` binding site.

**Explanation:** A borrowed ancilla appears in the block's result value, so
it would escape the borrow scope. An ancilla must be cleaned up (measured,
`reset`, or `discard`ed) inside the block, never returned. See [Borrow
blocks](../language/borrow/).

**Repair:** Do not return the borrowed name. Consume it inside the block with
`measure(a)`, `reset(a)`, or `discard(a)`, and return a different value. No
automated rewrite is offered — the correct fix depends on what the block
should produce.

---

## Elaboration and lowering

### Non-dependent argument

**Code:** `quon.dependent.non-dependent-arg`

**Diagnostic:**

```text
argument for the `Nat` parameter `n` of `repeat` is not a static depth
expression; use an `Int` literal, variable, or `+ - * / ^` over them
```

**Explanation:** A `Nat` value argument at a value-dependent call site could
not be lowered to a symbolic depth. Only `Int` literals, variables, and
`+ - * / ^` over them specialize a dependent parameter. This fires during
elaboration, after type-checking succeeds.

**Repair:** Use a static depth expression (literal, variable, or arithmetic
over them) for the `Nat` parameter.

---

### Ill-founded recursion

**Code:** `quon.recursion.ill-founded`

**Diagnostic:**

```text
cannot prove that the recursive function `repeat` terminates; some `Nat`
parameter must strictly decrease (and stay non-negative) at every recursive
call — add or adjust a base case so the recursion is well-founded
```

**Explanation:** A recursive circuit function whose recursion could not be
shown to terminate. Without a well-founded measure, the depth index is not a
bound on any finite circuit, so the function is rejected — and reported,
never looped on.

**Repair:** Ensure some `Nat` parameter strictly decreases (and stays
non-negative) at every recursive call. Add or adjust a base case.

---

### Mutual recursion

**Code:** `quon.recursion.mutual`

**Diagnostic:**

```text
`even` is part of a mutually-recursive cycle; only direct self-recursion
with a decreasing measure is supported
```

**Explanation:** Mutual recursion among circuit functions. Quon v1 supports
only direct self-recursion with an inferred decreasing measure; a cycle
through two or more distinct functions is rejected rather than accepted
without a termination witness.

**Repair:** Inline one function into the other, or restructure as direct
self-recursion. This is a v1 limitation — see [Limitations](#limitations).

---

### Unsupported quantum fragment

**Code:** `quon.unsupported.quantum-fragment`

**Diagnostic:**

```text
`while` is part of the linear/quantum fragment and is not yet type-checked
```

**Explanation:** A construct that belongs to the linear/quantum fragment was
encountered while type-checking the classical fragment. This indicates a
construct the v1 typechecker does not yet handle.

**Repair:** Use a supported construct. This is a v1 limitation — see
[Limitations](#limitations).

---

## QEC and target capability

Quon's QEC types (`QecBlock<F, d>`) are tracked by the type system alongside
bare `Qubit` values. See [QEC blocks](../language/qec/) for the full guide.

### Invalid QEC distance

**Code:** `quon.qec.invalid-distance`

**Failing program (repetition):**

```kotlin
fn f(): Q<Bit> = run {
    b <- repetition_code<1>()
    measure_logical(b)
}
```

**Diagnostic:**

```text
invalid distance 1 for repetition code; requires d ≥ 2
```

**Failing program (surface):**

```kotlin
fn f(): Q<Bit> = run {
    b <- surface_code<2>()
    measure_logical(b)
}
```

**Diagnostic:**

```text
invalid distance 2 for surface code; requires odd d ≥ 3
```

**Explanation:** QEC constructor distance violates family rules. Repetition
codes require `d ≥ 2`; surface codes require odd `d ≥ 3`.

**Repair:** Use a valid distance for the code family.

---

### Mixed QEC entrypoint

**Code:** `quon.qec.mixed-entrypoint`

**Diagnostic:**

```text
entrypoint mixes QecBlock with bare Qubit/QReg; use one encoding style per program
```

**Explanation:** An entrypoint mixes `QecBlock` with bare `Qubit`/`QReg`
(ADR-0014). A program must use one encoding style — either all bare qubits
or all QEC blocks.

**Repair:** Use a single encoding style throughout the entrypoint.

---

### Unknown code family

**Code:** `quon.qec.unknown-family`

**Diagnostic:**

```text
unknown code family `Color`; v1 supports `Repetition`, `Surface`,
or an in-scope `F: CodeFamily` parameter
```

**Explanation:** A `CodeFamily` tag is not in the closed v1 set
(`Repetition`, `Surface`) and is not an in-scope type parameter.

**Repair:** Use `Repetition` or `Surface`, or introduce a type parameter
`F: CodeFamily`.

---

### Logical CX requires surface

**Code:** `quon.qec.logical-cx-family`

**Diagnostic:**

```text
`logical_cx` requires surface-code blocks at equal distance;
found family `Repetition`
```

**Explanation:** `logical_cx` requires both arguments to be surface-code
blocks. Repetition-code blocks do not support transversal CNOT.

**Repair:** Use surface-code blocks for `logical_cx`.

---

### Logical CX distance mismatch

**Code:** `quon.qec.logical-cx-distance`

**Diagnostic:**

```text
`logical_cx` requires equal distances; expected `3`, found `5`
```

**Explanation:** `logical_cx` requires equal distances on both surface
blocks. Mismatched distances would produce an invalid logical operation.

**Repair:** Use surface-code blocks at the same distance.

---

### Non-Clifford requires surface

**Code:** `quon.qec.nonclifford-family`

**Diagnostic:**

```text
`logical_t` requires a surface-code block; found family `Repetition`
```

**Explanation:** A magic-state non-Clifford operation (`logical_t`,
`logical_tdag`, `logical_ccz`) requires a surface-code block.

**Repair:** Use surface-code blocks for non-Clifford logical operations.

---

### Non-Clifford distance mismatch

**Code:** `quon.qec.nonclifford-distance`

**Diagnostic:**

```text
`logical_ccz` requires surface-code blocks at equal distance;
expected `3`, found `5`
```

**Explanation:** `logical_ccz` requires three surface-code blocks at equal
distance.

**Repair:** Use three surface-code blocks at the same distance.

---

### QEC constructor requires distance

**Code:** `quon.qec.ctor-requires-distance`

**Failing program:**

```kotlin
fn f(): Q<Bit> = run {
    b <- repetition_code()
    measure_logical(b)
}
```

**Diagnostic:**

```text
`repetition_code` requires a distance type argument, e.g. `repetition_code<3>()`
```

**Explanation:** A QEC constructor was used without a distance type argument.

**Repair:** Supply a distance: `repetition_code<3>()`.

---

### Non-literal QEC distance

**Code:** `quon.qec.non-literal-distance`

**Diagnostic:**

```text
QEC constructor distance must be a literal Nat (e.g. `repetition_code<3>()`)
```

**Explanation:** QEC constructor distance is not a literal `Nat`. Deferred
specialization (distance determined at runtime) is not supported in v1.

**Repair:** Use a literal `Nat` for the distance. This is a v1 limitation —
see [Limitations](#limitations).

---

## Backend target and artifact emission

These errors arise from the backend crate (`backend::error::BackendError`)
and the `quonc` CLI driver when a target descriptor is invalid or an emission
flag is incompatible with the target kind. See [Backends and
verification](../guides/backends/) and the [quonc CLI reference](../quonc/).

### Target descriptor errors

| Code / message | Cause | Repair |
|---|---|---|
| `qubit index {got} out of range (num_qubits = {n})` | A gate in the descriptor references a qubit `≥ num_qubits`. | Fix the index or increase `num_qubits`. |
| `edge ({a}, {b}) references a qubit >= num_qubits` | A connectivity edge names an endpoint outside `0..num_qubits`. | Fix the edge endpoints. |
| `self-loop edge on qubit {0} is not allowed` | A connectivity edge connects a qubit to itself. | Remove the self-loop. |
| `duplicate connectivity edge ({a}, {b})` | A connectivity edge duplicates an undirected pair already present. | Remove the duplicate. |
| `unknown native gate `{0}` (no decomposition registered)` | A native gate name has no registered decomposition. | Register the gate or remove it from `native_gates`. |
| `unknown target kind `{0}`` | A target descriptor names an architecture family the backend does not recognize. | Use `fixed` or `neutral_atom_reconfigurable`. |
| `invalid target configuration: {0}` | The descriptor is syntactically valid JSON but violates semantic invariants (positive geometry, non-overlapping zones, capacity limits). | Fix the named invariant. |
| `malformed two-qubit noise key `{0}` (expected "u,v")` | A two-qubit noise key was not of the form `"u,v"`. | Use the `"u,v"` format. |
| `malformed qubit-index key `{0}` in noise model` | A noise-map key was expected to be a qubit index but did not parse. | Use a numeric qubit index. |

### Missing error model

**Diagnostic:**

```text
neutral-atom target is missing error_model required for QEC error reporting
(--emit-resource-report) or --emit-qec-experiment; set error_model on the
target (do not derive from fidelity)
```

**Explanation:** `--emit-resource-report` (with error budget) or
`--emit-qec-experiment` was requested, but the neutral-atom target has no
`error_model`. The compiler never invents defaults or derives rates from
`fidelity` (ADR-0017).

**Repair:** Set `error_model` on the target descriptor. See the neutral-atom
[targets directory](https://github.com/arniber21/quon/tree/main/targets/neutral_atom)
for examples.

### Emission flag / target mismatch

| Diagnostic | Cause | Repair |
|---|---|---|
| `--emit-qasm requires a fixed (gate-model) target; use --emit-na-mlir ... for neutral-atom targets` | `--emit-qasm` was passed with a neutral-atom target. | Use `--emit-na-mlir` or switch to a fixed target. |
| `--emit-na-mlir / ... require a neutral_atom_reconfigurable target` | An NA emission flag was passed with a fixed target. | Use a neutral-atom target or switch to `--emit-qasm`. |
| `--verify-na requires a neutral_atom_reconfigurable target` | `--verify-na` was passed with a fixed target. | Use a neutral-atom target. |
| `--emit-qec-experiment requires a filesystem PATH` | `--emit-qec-experiment` was given `-` (stdout). | Supply a file path. |
| `--emit-qec-validation requires a filesystem PATH` | `--emit-qec-validation` was given `-` (stdout). | Supply a file path. |
| `--emit-naviz requires a filesystem PATH` | `--emit-naviz` was given `-` (stdout). | Supply a file path. |
| `OpenQASM emission produced no output (is the target fixed?)` | The QASM emitter returned empty for a non-fixed target. | Use a fixed target. |

### Linearity verification failure (lowering)

**Diagnostic:**

```text
linearity verification failed:
<rendered MLIR diagnostics>
```

**Explanation:** After lowering to MLIR, the linearity verifier
(`mlir_bridge`) re-checks resource ownership on the IR. This catches issues
the static typechecker cannot see — typically involving dynamic `run` blocks
with measurement-dependent control flow.

**Repair:** Read the rendered MLIR diagnostics, which name the offending
operation and resource. The fix is usually the same as for static linearity
errors: ensure every resource is consumed exactly once on every path.

---

## Limitations

Some diagnostics have no supported automated rewrite. These are tracked as
v1 limitations and will be addressed in future milestones. The [maturation
path](../guides/roadmap/) tracks the overall status.

| Diagnostic | Limitation | Workaround |
|---|---|---|
| `quon.linearity.discard` (in `let ... in`) | No auto-fix outside `run` blocks — the safe rewrite depends on context. | Use `discard(q)` or `measure(q)` explicitly. |
| `quon.borrow.escape` | No auto-fix — the correct fix depends on what the block should produce. | Consume the ancilla inside the block and return a different value. |
| `quon.refinement.depth-intractable` | The solver cannot verify the constraint; no rewrite. | Supply a static depth bound. |
| `quon.recursion.mutual` | v1 supports only direct self-recursion. | Inline one function or restructure as self-recursion. |
| `quon.qec.non-literal-distance` | Deferred specialization is not supported in v1. | Use a literal `Nat` distance. |
| `quon.unsupported.quantum-fragment` | The construct is not yet type-checked. | Use a supported construct. |
| `quon.parse.unexpected-token` | Single default bucket — no per-rule parse error codes. | Read the token and surrounding context. |
| Line comments | Not yet lexed — `//` and `#` are not comment openers. | Use block comments `{- ... -}`. |
