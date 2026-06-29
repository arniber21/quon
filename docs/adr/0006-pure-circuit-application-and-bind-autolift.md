# Circuit application `@` is pure unitary; `<-` auto-lifts a pure quantum resource

Outside a `circuit { }` block, `c @ r` (circuit application, SPEC §5.9 `apply`) is a **pure**
linear operation, not a monadic one:

| register source `r` | result        |
|---------------------|---------------|
| `QReg<n>`           | `QReg<m>`     |
| `Qubit` (n = 1)     | `Qubit`       |
| `Q<QReg<n>>`        | `Q<QReg<m>>`  |

A unitary has no measurement, so applying one to a register just transforms a handle; the `Q`
state monad threads the underlying quantum state implicitly (SPEC §3.5). Only when the register
*source* is itself monadic (`Q<QReg<n>>`, e.g. `qreg(2)`) does the result re-enter `Q`.

Correspondingly, a monadic bind `x <- e` accepts `e : Q<A>` (the usual case) **or** a pure
quantum resource `e : A` where `A` is a `Qubit`/`QReg`/`Circuit`, which is *auto-lifted* (bound
directly, as `let` would). A pure *classical* value (`x <- 5`) or an unsolved type is rejected
as `TypeError::ExpectedMonad`. The monad is entered by `measure`/`qreg`/`reset`/`discard`/
`return`, never by a pure value.

Implemented in `apply_circuit` and `synth_bind` (`frontend/src/typecheck/mod.rs`).

## Context

The two reference algorithms (#14 acceptance criteria) pin the semantics, and only one reading
satisfies both:

`hello_bell` binds a **monadic** source — `qreg(2) : Q<QReg<2>>`, so
`(q0, q1) <- bell_state() @ qreg(2)` requires `@` to thread `Q` and yield `Q<QReg<2>>`.

`teleport` binds a **pure** source — `(a, b) <- bell_state() @ (alice, bob)` where
`(alice, bob) : QReg<2>` is pure — *and* applies circuits in `let`s:

```kotlin
let b2 = (if x_bit then X else identity(1)) @ b   -- b : Qubit
let b3 = (if z_bit then Z else identity(1)) @ b2
return b3                                          -- must be Q<Qubit>
```

For `return b3 : Q<Qubit>` to hold, `b3` (hence `X @ b`) must be a **pure `Qubit`**. If `@`
were monadic, `b2`/`b3` would be `Q<…>` and `return b3` would produce `Q<Q<…>>` — a type
error. So `@` must be pure. But then `teleport`'s first bind has a pure `QReg<2>` on the
right of `<-`, which only type-checks if `<-` auto-lifts a pure quantum resource.

## Considered Options

**`@` always monadic (`Circuit @ reg : Q<QReg<m>>`), `<-` strictly requires `Q<_>`.** The
clean monadic reading and a literal fit for #14's AC5 ("a `Bind` whose `e₁` is not `Q<_>`
errors"). Rejected: it makes `teleport`'s correction `let`s produce `Q<QReg<1>>`, so
`return b3` double-wraps to `Q<Q<…>>` and the fixture cannot type-check without rewriting the
reference algorithm (which we declined to do, as with ADR-0005).

**`@` pure with monadic threading; `<-` auto-lifts a pure quantum resource (chosen).** Both
fixtures type-check unmodified. AC5 is honored in spirit: a `<-` on a non-quantum value
(classical, or unresolved) is still `ExpectedMonad`; the relaxation is narrow and principled —
a pure register/qubit is genuinely part of the threaded quantum state, so naming it with `<-`
is meaningful, whereas `x <- 5` is not.

## Consequences

- A single-qubit gate on a `Qubit` returns a `Qubit` (not `QReg<1>`), so qubit handles stay
  `Qubit` through a chain of corrections — required for `teleport`'s `return b3 : Q<Qubit>`.
- `<-` and `let` coincide for a pure quantum resource; the distinction only matters for `Q<_>`
  sources, which `let` would leave wrapped. This is a deliberate ergonomic overlap.
- `ExpectedMonad` fires for `x <- <classical>` and for an unresolved right-hand side, giving
  #14's AC5 a concrete, span-accurate error while leaving the quantum cases permissive.
- `if`/`match` conditions accept `Bit` as well as `Bool` (a measured bit drives classical
  control, e.g. `if x_bit then X else identity(1)`), consistent with this monadic fragment.
