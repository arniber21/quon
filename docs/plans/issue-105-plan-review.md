# Adversarial review — Issue #105 entangling-layer schedule plan

**Plan under review**: `docs/plans/issue-105-plan.md`  
**Verdict**: APPROVED FOR IMPLEMENTATION (after fixes below incorporated)

## Attack surface

### A1. Claiming Enola Thm. 1 while shipping plain greedy

**Attack**: Issue title still says “greedy edge-coloring”; implementing ≤ 2Δ−1 greedy while docs cite Misra–Gries / ≤ Δ+1 is the exact attribution slip #99 warned about.

**Resolution**: Plan mandates Misra–Gries for `CommutationGroup` 2Q graphs; docs must name Misra–Gries; Enola Thm. 1 only when that path runs. `misra_gries_applied` flag records whether the claim holds for the run.

**Status**: Fixed in plan. ✅

### A2. Conflict graph vs interaction graph

**Attack**: Edge-coloring the *interaction* graph (qubits = vertices, gates = edges) is correct for 2Q commutation groups. Building a *gate-conflict* graph and vertex-coloring it is a different algorithm (also valid, but not Misra–Gries / not Enola’s reduction).

**Resolution**: Plan uses interaction-graph edge coloring for 2Q commutation segments. DependencyDag uses ASAP, not coloring. Multi-edge / duplicate-pair fallback must not claim Misra–Gries.

**Status**: OK. ✅

### A3. Multi-qubit gates in commutation groups

**Attack**: k>2 gates are hyperedges; Misra–Gries does not apply. Silently treating them as cliques of pairs would over-constrain and mis-cite.

**Resolution**: Error `MultiQubitCommutation` for arity ≠ 2 in commutation segments. k>2 belongs in DependencyDag (EntangleN + ASAP).

**Status**: Fixed. ✅

### A4. Capacity vs chromatic bound

**Attack**: Cubic Δ=3 ⇒ ≤ 4 colors, but capacity=1 would force many more layers. Tests must use capacity ≥ matching size (e.g. 340) for the ≤ 4 assertion.

**Resolution**: Cubic regression uses large capacity; separate test covers capacity splits. Document that Enola stage bound is pre-capacity; capacity can only increase stage count.

**Status**: Must assert in test comments. ✅ **Plan amendment**: cubic test documents “pre-capacity chromatic bound; capacity=340”.

### A5. AtomId without layout

**Attack**: Graph-only scheduling needs atom ids; placement may be absent.

**Resolution**: `AtomId(LogicalQubitId.0)` — same identity as #104. Layout left unchanged (`None` OK).

**Status**: OK. ✅

### A6. Overwriting existing layers

**Attack**: `schedule_entangling_layers` on a request that already has layers — silent overwrite?

**Resolution**: Always overwrite `layers` (pure function of graph + capacity). Leave `layout` as-is. Document like `place()`.

**Status**: Acceptable. ✅ **Plan amendment**: document overwrite semantics.

### A7. Segment order and global cycles

**Attack**: Concatenating segment schedules may insert artificial barriers between commutation groups that could have been merged.

**Resolution**: In-scope: process segments in order (matches barrier-bounded extraction). Cross-segment merging is #108 compaction territory.

**Status**: Out of scope / OK. ✅

### A8. Determinism / flaky color counts

**Attack**: Misra–Gries color *assignment* can vary with edge order while respecting ≤ Δ+1; asserting exact color count = χ′ may flake.

**Resolution**: Assert `layers.len() ≤ Δ+1` (after capacity) for cubic, not exact χ′. Optional: assert `max_degree == 3` for cubic.

**Status**: Fixed. ✅

### A9. Empty graph / no interactions

**Attack**: Empty interactions → empty layers, utilization empty, `misra_gries_applied = false`.

**Resolution**: Return Ok with empty layers; no error.

**Status**: OK. ✅

### A10. `schedule_from_graph` contract break

**Attack**: Changing stub to auto-schedule breaks existing proptests expecting empty layers.

**Resolution**: Keep stub; new `schedule_entangling_layers` API. Update proptests separately for the new entry point.

**Status**: Fixed. ✅

### A11. Flux spec vacuity

**Attack**: Weak Flux specs that always hold.

**Resolution**: Spec `capacity_layer_count` as `ceil_div` style with `cap > 0` and result ≥ 1 when n > 0 (mirror `qec::ceil_div`).

**Status**: OK. ✅

### A12. Scope creep (AOD, zones, durations)

**Attack**: Pulling movement or real timing into this slice.

**Resolution**: Entangle-only layers; duration placeholder 1 µs; no moves/transfers.

**Status**: Out of scope. ✅

## Amendments incorporated into implementation

1. Cubic ≤ 4 test uses capacity=340 and comments that the bound is chromatic (pre-capacity).
2. `schedule_entangling_layers` always overwrites `layers`; preserves `layout`.
3. Duplicate same-pair interactions in a commutation group: reject with a typed error (simpler than silent non-MG fallback) — **prefer reject** for v0 so `misra_gries_applied` stays honest.

## Verdict

**APPROVED** — proceed to implement Misra–Gries + ASAP + capacity-capped emission in `quon_na`.
