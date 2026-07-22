# Collapse the `monadic_staging` dialect into `quantum.dynamic`

## Status

Accepted — implemented in #213.

## Context

CONTEXT.md lists two IR dialects — `quantum.circ` (purely unitary) and
`quantum.dynamic` (measurement, reset, feed-forward, embedded unitary islands) —
plus "physical-as-attributes" (`quantum.dynamic` ops carrying `phys_qubit` /
`native_gate` / `fidelity`). A third, undocumented dialect was leaking into the
code: `monadic_staging`.

`monadic_staging` ops lived under `quantum.circ.*` names (`quantum.circ.run`,
`qreg`, `apply`, `cond_apply`, `measure`, `reset`, `discard`, `yield`, and the
`qec_*` staging ops). They were produced by `frontend::lower` inside a
`quantum.circ.run` region, then erased by the `monadic_lowering` pass, which
rewrote them to `quantum.dynamic` IR (`test.qubit` allocations, `measure` /
`reset`, `unitary_region` / `if` with inlined circuit callees, `qec_dynamic`
ops). The staging ops "never appear in final IR" — their only consumer was the
erasure pass. Readers discovered a third dialect by accident, and issue #213
asked to either (A) name a staging dialect, or (B) collapse staging into
`quantum.dynamic` directly.

## Decision

**Option B — Collapse.** `frontend::lower` now lowers `run { }` blocks straight
into `quantum.dynamic` IR (and `unitary_region` / `if` bodies). The
`monadic_staging` dialect module, the `monadic_lowering` pass, the
`monadic_lower` example binary, and the staging-only lit/Integration tests are
deleted. There is no staging dialect and no erasure pass.

### Why collapse

Staging was ephemeral glue: it added a pass (`monadic_lowering`) whose only job
was to undo staging ops the frontend had just emitted, with no semantic value.
The `run` / `apply` / `qreg` staging ops mapped 1:1 to `quantum.dynamic` ops
that `lower.rs` could emit directly. The deletion test was decisive: staging
never appeared in final IR and its only consumer was the erasure pass — dead
weight.

### How the lowering moved into `lower`

`lower_run_fn` walks the desugared monadic body (`Bind` / `Let` / `Return` chain,
issue #8) and appends `quantum.dynamic` ops to the module's top-level block
directly:

| Source construct           | Emitted IR (direct)                                   |
|----------------------------|-------------------------------------------------------|
| `qreg(n)`                  | `n` × `test.qubit`                                    |
| `measure(q)` / `measure_all` | `quantum.dynamic.measure`                           |
| `C @ qs` (apply)           | `quantum.dynamic.unitary_region` with `C`'s body inlined |
| `if b then C₁ else C₂ @ qs` | `quantum.dynamic.if` with both branches inlined      |
| QEC builtins               | `quantum.dynamic.qec_*`                              |

Circuit callees are inlined into `unitary_region` / `if`-branch bodies by
re-lowering the callee's recorded AST body through the **existing**
`lower_circuit_block` / `lower_circuit_body_expr` walker — the single source of
truth for circuit-body lowering — so the inlined gates match the emitted
`quantum.circ.func` definition exactly. Op results are extracted from the
still-unparented owned operation *before* it is appended, so they carry the
context lifetime and thread through the run-block environment across appends.

### Circ passes now reach `unitary_region` / `if` bodies

In the old flow, the circ optimization passes (`gate_cancellation`,
`rotation_merging`, `zx_simplification`) ran on `quantum.circ.func` definitions
*before* `monadic_lowering` inlined (the now-optimized) bodies into
`unitary_region`s. Collapsing inlines *during* `lower`, i.e. before the circ
fixpoint — so the circ passes must now descend into `quantum.dynamic.unitary_region`
bodies (and `quantum.dynamic.if` branch bodies) to optimize the gates that live
there. `gate_cancellation` and `rotation_merging` run their block-level logic on
those bodies (a `quantum.circ.return` / `quantum.dynamic.yield` terminator is
skipped); `zx_simplification` runs `simplify_func` on `unitary_region` ops
(single-qubit, `return`-terminated). `compiler_uncomputation` is unchanged: it
only ever walked top-level `quantum.circ.borrow` ops.

This is the architecturally correct shape for the collapsed model: in the final
IR, circ gates live in `unitary_region` / `if` bodies, so circ optimization must
reach them there rather than only in (now-dead-after-inlining) `func`
definitions.

## Alternatives considered

- **Option A — Name a staging dialect.** Give `monadic_staging` a real
  namespace and document it. Rejected: it codifies a dialect whose sole purpose
  is to be erased, and leaves a pass with no semantic value in the pipeline.

- **Keep `monadic_lowering` as a thin post-circ inlining pass** (emit
  `unitary_region`s referencing a callee, optimize `func` defs, then inline).
  Rejected: this is the staging pass under another name, and the direct
  lowering is strictly simpler.

## Consequences

- Two documented IR dialects remain: `quantum.circ` and `quantum.dynamic`
  (physical-as-attributes). No third, undocumented dialect.
- `--dump-ir` no longer has an "after monadic lowering" stage; the stage list is
  `lower → circ fixpoint → dynamic passes → physical`.
- Circ passes optimize `unitary_region` / `if` bodies directly; the
  `quantum.circ.func` definitions remain as dead code after inlining (skipped by
  the dynamic walkers, as before).
- Follow-up (#206, #201): the specialized-circuit and unitary-region paths build
  on this direct lowering.

## References

- Issue #213 — name or erase `monadic_staging`.
- Issue #201, #206 — dependent lowering paths.
- ADR-0004 — `quantum.circ` as an unregistered dialect (the staging ops reused
  its `quantum.circ.*` namespace; this ADR removes that reuse).
