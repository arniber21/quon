# Adversarial review — Issue #104 placement plan

**Plan under review**: `docs/plans/issue-104-plan.md`  
**Verdict**: APPROVED FOR IMPLEMENTATION (after fixes below incorporated)

## Attack surface

### A1. Score definition vs issue wording

**Attack**: Issue says “total expected movement distance”; comment says prefer `Σ √dist` not Manhattan; user query says `weight * site_distance`.

**Resolution**: Use `Σ_e w(e) * sqrt(euclidean_um(e))`. Matches architecture_model movement law and issue comment. Document that raw Manhattan is rejected. User-query “site_distance” is satisfied by Euclidean under the √-law wrapper.

**Status**: Fixed in plan. ✅

### A2. Clustering may not beat row-major

**Attack**: Greedy agglomerative clustering on cubic/ER graphs may not improve score; acceptance requires ≥1 benchmark.

**Resolution**: Dedicated synthetic 2-clique graph (two K₄ + one bridge) guarantees clustering packs cliques contiguously while row-major interleaves ids across the cut. Do not rely on random ER alone.

**Status**: Fixed (`clustering_beats_row_major`). ✅

### A3. Atomique MAX-k-Cut inverted incorrectly

**Attack**: Atomique maximizes cut across *arrays*; naively copying MAX-k-Cut on a single grid would *separate* interacting qubits and *worsen* our score.

**Resolution**: Plan already adapts to **maximize intra-cluster weight** (community / min spatial cost), documented as inspiration not reproduction. Spiral degree-based remains the load-balance echo.

**Status**: OK if module docs are explicit. ✅

### A4. Spiral on non-square grids

**Attack**: Center spiral can skip cells or revisit on `rows ≠ cols`.

**Resolution**: Generate spiral candidates; filter to in-bounds unique cells; if fewer than `n` cells collected, append remaining cells in row-major. Deterministic, overlap-free.

**Status**: Must implement carefully. ✅

### A5. Empty / isolated vertices

**Attack**: Graphs with vertices but no edges; or isolated vertices in ER.

**Resolution**: Score 0 for no edges; isolates still get unique sites. Empty vertex list → `EmptyGraph`.

**Status**: Covered. ✅

### A6. Overwriting existing layout

**Attack**: `place` on a request that already has `layout: Some` — silent overwrite?

**Resolution**: Always overwrite (placement is pure function of graph+strategy). Document. No error needed for v0.

**Status**: Acceptable. ✅

### A7. Site count vs qubit count

**Attack**: `rows*cols >= n` but bindings only for `n` — unused sites OK?

**Resolution**: Yes; layout may list all grid sites; bindings length == n. Overlap check on bindings only.

**Status**: OK. ✅

### A8. Non-contiguous LogicalQubitId

**Attack**: Vertices `{0, 2, 5}` — AtomId mapping by `.0` leaves gaps; site count uses `vertices.len()` not `max_id+1`.

**Resolution**: Place exactly `vertices.len()` atoms; `AtomId(logical.0)` preserves id identity; sites are dense `0..capacity`. Score looks up by LogicalQubitId → AtomId → site.

**Status**: Must use vertex list, not assume `0..n-1` dense. ✅ **Plan amendment**: sort/iterate `graph.vertices`, never assume contiguous ids.

### A9. Floating-point score comparison

**Attack**: `clustering < row_major` flaky on near-ties.

**Resolution**: Synthetic clique graph should show clear gap (e.g. > 1% or absolute delta). Assert `cluster_score < row_score` with margin `1e-6` relative or construct graph where gap is obvious.

**Status**: OK. ✅

### A10. Scope creep (AOD, zones, SA)

**Attack**: Temptation to model AOD bindings or Enola SA.

**Resolution**: SLM-only bindings; three heuristics only. Out of scope section stands.

**Status**: OK. ✅

### A11. Flux overclaim

**Attack**: Fake Flux specs that prove nothing.

**Resolution**: Either a real `grid_site_count(rows, cols) >= n` style helper with a meaningful spec, or skip Flux and say N/A in PR. Prefer a tiny pure `fn grid_dims(n) -> (rows, cols)` with `rows * cols >= n` if Flux-friendly; else omit.

**Status**: Prefer omit unless trivial. ✅

### A12. Plan doc in PR?

**Attack**: Large plan markdown may be noise.

**Resolution**: Keep plan in repo (matches other issues); include in commit. Fine.

**Status**: OK. ✅

## Amendments applied to implementation checklist

1. Iterate `graph.vertices` (non-contiguous ids safe).
2. Spiral: fill gaps with leftover row-major cells.
3. Clustering objective = maximize intra-cluster weight, then pack bands.
4. Score = `Σ w √(euclid)`; synthetic clique for improvement proof.
5. No AOD; SLM only.
6. Flux optional / skip if not meaningful.

## Final verdict

**APPROVED FOR IMPLEMENTATION**
