# Adversarial code review — Issue #105

**Branch**: `issue-105`  
**Plan**: `docs/plans/issue-105-plan.md` (APPROVED)  
**Verdict**: APPROVED TO SUBMIT (with notes)

## Spec compliance

| Criterion | Status |
| --------- | ------ |
| Misra–Gries for CommutationGroup 2Q | ✅ `misra_gries_color` + always-invert-then-rotate |
| ASAP for DependencyDag | ✅ `asap_buckets` by `dag_layer` |
| Capacity split | ✅ `chunks(max_parallel_entangling_pairs)` |
| Graph-only / layout optional | ✅ layout preserved; `AtomId(q.0)` |
| `validate_conflicts` | ✅ every emitted layer |
| Cubic Δ=3 → ≤4 layers | ✅ unit + proptest |
| Utilization reported | ✅ `LayerUtilization` |
| Attribution (not plain greedy) | ✅ module docs + literature_notes |

## Attacks checked

1. **Wrong algorithm labeled Misra–Gries** — Core matches Wikipedia/Misra–Gries: maximal fan, invert cd-path (first edge color `d` because `c` free at center), rotate prefix, color with `d`. Internal `validate_coloring` asserts proper ≤Δ+1 coloring.
2. **Capacity inflating cubic bound** — Tests use capacity=340 and document pre-capacity chromatic bound.
3. **Stub contract break** — `schedule_from_graph` still returns empty layers; new API is `schedule_entangling_layers`.
4. **Multi-qubit / duplicate pairs** — Typed errors; no false Enola claim.
5. **Conflict in color class** — Would fail `validate_conflicts`; covered by cubic/ER props.

## Residual risks

- Fan construction iterates `BTreeMap` neighbors (deterministic) but order differs from some textbook presentations; bound still holds.
- Entangle duration is placeholder `1` µs (in-scope).
- Flux spec on `capacity_layer_count` is present; full `cargo flux` not required unless CI path enabled for this crate change alone.

## Literature check

- architecture_model §4 already cites Misra–Gries / Enola Thm. 1.
- literature_notes updated to say Misra–Gries (not “greedy edge-coloring”) for #105.
