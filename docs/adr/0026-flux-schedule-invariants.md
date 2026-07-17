# Flux refinement types for `quantum.na` schedule invariants

## Status

Accepted (research note — issue #115)

## Context

The `quantum.na` dialect verifier (`quon_na/src/dialect.rs`, ~1900 LOC) checks
~15 structural and schedule-legality invariants on neutral-atom schedule IR. The
workspace already uses Flux (`flux-rs`) refinement types on pure MLIR-free
kernels in `quon_core` (linearity, optimization depth bounds, QASM index/arity)
behind a `#[cfg(feature = "flux")]` gate. Issue #115 asks whether the `quantum.na`
verifier invariants can be similarly refined, and whether this is worth doing.

### Flux's expressiveness boundary

Flux refinement types add first-order predicates to Rust function signatures:

```
#[spec(fn(prev: u32, curr: u32) -> bool[prev <= curr])]
```

Flux handles **pure functions over integers and booleans** — linear arithmetic,
comparisons, conjunction/disjunction. It does **not** support:

- **Floating-point arithmetic** (`f64` comparisons, `sqrt`, `abs`)
- **Collection operations** (`BTreeSet::insert`, `BTreeMap::get`, `Vec::contains`)
- **Loops over variable-length data** (it can reason about indexed accesses but
  not arbitrary iteration with accumulators)
- **Enums with payload-carrying state machines** across function boundaries

### Verifier invariants classified

| # | Invariant | Type | Flux? | Reason |
|---|-----------|------|-------|--------|
| 1 | Cycle monotonicity (non-decreasing) | `u32` comparison | ✅ | Pure integer `prev <= curr` |
| 2 | Wait barrier (strictly later after Wait) | `u32` comparison | ✅ | Pure integer `wait_cycle < after_cycle` |
| 3 | Atom ID non-negativity | `u32` (already unsigned) | ✅ | Trivially `0 <= v` |
| 4 | Measurement ordering (same-cycle conflict) | Set membership | ❌ | Requires `BTreeSet::contains` |
| 5 | Measure→reset→reuse state machine | Enum + collection | ❌ | `BTreeMap<u32, AtomPhase>` + enum state |
| 6 | Double-measure without reset | Enum + collection | ❌ | Same as #5 |
| 7 | Reset-use same cycle | Set membership | ❌ | `BTreeSet::intersection` |
| 8 | Reset-before-measure | Enum + collection | ❌ | Cross-cycle state |
| 9 | Occupancy (unique atom per cycle) | `BTreeSet::insert` | ❌ | Set membership return value |
| 10 | Occupancy (unique site per cycle) | `BTreeSet::insert` | ❌ | Same |
| 11 | Duplicate entangling atom | `BTreeSet::insert` | ❌ | Set membership |
| 12 | Entangling pair range (R1) | `f64` distance | ❌ | `sqrt(dx² + dy²)` comparison |
| 13 | Compulsory entanglement (R2) | `f64` distance | ❌ | Same |
| 14 | Rydberg spacing (R3) | `f64` distance | ❌ | Same |
| 15 | AOD trap consistency | `BTreeMap` lookup | ❌ | Map lookup + tuple equality |
| 16 | AOD trap double-claim | `BTreeMap` lookup | ❌ | Map lookup + `f64` equality |
| 17 | AOD row/column coupling | `f64` delta | ❌ | `f64` subtraction comparison |
| 18 | AOD order preservation | `f64` comparison | ❌ | `f64::total_cmp` |
| 19 | AOD separation | `f64` distance | ❌ | `f64::abs` comparison |
| 20 | Region/operand arity | `usize` comparison | ✅ | Pure `operands == results` |
| 21 | JSON attribute structure | serde | ❌ | Not a refinement concern |

### Summary

**3 invariants are Flux-expressible** (#1 cycle monotonicity, #2 Wait barrier,
#3 atom ID non-negativity). These are all simple integer comparisons that the
verifier already checks in Rust.

**18 invariants are NOT Flux-expressible** because they require floating-point
arithmetic (geometry checks), collection operations (occupancy, entangle
uniqueness, trap consistency), or state-machine tracking across layers
(measurement/reset lifecycle).

## Decision

**Prototype cycle monotonicity** as a Flux refinement-typed kernel — it is the
strongest candidate: a real schedule invariant, pure integer logic, and
directly maps to the existing pattern in `quon_core`.

**Do NOT invest further** in Flux-refining the geometry, occupancy, or
measurement-lifecycle invariants. The ROI is negative:

- **Geometry invariants** (R1–R3, AOD coupling/order/separation) need `f64`
  arithmetic that Flux cannot express. You would need to model positions as
  fixed-point integers (e.g. nanometres as `i64`) and rewrite all distance
  calculations — a major refactor for no verification gain beyond what the
  runtime verifier already provides.
- **Collection invariants** (occupancy, entangle uniqueness, trap consistency)
  need set/map operations Flux cannot reason about. You would need to model
  these as sorted arrays with binary-search predicates — possible in theory but
  not worth the complexity.
- **State-machine invariants** (measurement lifecycle) need cross-layer
  accumulation that is beyond Flux's current scope.

The prototype confirms Flux *can* catch a constructed violation of cycle
monotonicity, but the same violation is already caught by the runtime verifier.
Flux adds compile-time proof for a narrow class of scalar invariants; the
high-value `quantum.na` invariants live outside that class.

### Recommendation

**Not worth pursuing further.** The 3 Flux-expressible invariants are trivially
correct Rust (`prev <= curr`), and the 18 valuable invariants are out of Flux's
reach. The existing runtime verifier in `dialect.rs` is the right tool — it
handles `f64`, collections, and state machines natively. The Flux prototype is
kept as a regression test pinning the kernel, but no further Flux investment is
warranted for `quantum.na`.

If the geometry checks were ever moved to fixed-point integer arithmetic (e.g.
for deterministic cross-platform reproducibility), the calculus would change:
R1–R3 and AOD order/separation would become Flux-checkable. That is a
prerequisite, not a follow-up.

## Considered Options

**Option A — Full Flux coverage of all `quantum.na` invariants.** Rejected: 18
of 21 invariants use `f64` or collections that Flux cannot reason about. Would
require a fixed-point rewrite and sorted-array modeling — high effort, low
marginal assurance over the runtime verifier.

**Option B — Flux only on the scalar kernels, keep runtime verifier for the
rest.** Accepted: prototype `cycle_is_monotonic` and `wait_barrier_ok` as Flux
refinement-typed kernels. These pin the invariant at compile time and serve as
documentation, even though the runtime verifier is the real safety net.

**Option C — No Flux, rely entirely on the runtime verifier.** Rejected for this
issue: the issue explicitly asks for a prototype to evaluate feasibility. The
prototype is the deliverable that answers the question.

## Consequences

- A new MLIR-free module `quon_na/src/schedule_invariants.rs` holds the
  Flux-refined scalar kernels (`cycle_is_monotonic`, `wait_barrier_ok`).
- The kernels are unit-tested and pinned in `flux_verify/src/lib.rs` smoke
  tests, matching the existing pattern.
- No change to `dialect.rs` — the runtime verifier remains authoritative.
- Future Flux work on `quantum.na` should wait until/unless geometry moves to
  fixed-point arithmetic.
