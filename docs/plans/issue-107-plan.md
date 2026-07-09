# Issue #107 — Zoned routing-aware placement (RAP)

**Branch**: `issue-107` (stacked on `issue-105`)  
**Worktree**: `/Users/arnabghosh/projects/quon/.worktrees/issue-107`  
**Paper**: Stade et al., ICCAD 2025, arXiv:2505.22715 [RAP]

## Goal

Layer-by-layer zoned placement where **placement cost = routing cost** (RAP Sec. III-B / IV-A): greedily group compatible moves, charge Eq. (1) `Σ_G √(d_max(G))`, optionally with reuse (Sec. III-A). Ship both **routing-aware** and **routing-agnostic** (ZAC-style) modes. Enforce entanglement-zone-only 2Q ops and zone capacity.

## Soft-unblock vs #106

Flat AOD (#106) and RAP (#107) are distinct literature lines (architecture_model §4). This slice implements RAP-local movement grouping + √(d_max) costs sufficient for zone transfers; does not wait for #106's Enola sortIS planner.

## Design (smallest citable cut)

### Types in `quon_na` (MLIR-free)

`ZonedArchitecture` — serializable zone list + movement params (mirrors target schema fields needed here; no hard `backend` dep).

`PlacerMode::{ RoutingAware | RoutingAgnostic`

`ZonedScheduleResult` — updated `GraphScheduleRequest` (layout + layers with transfers/moves/entangles) + `routing_cost` + `rearrangement_steps`.

### Pipeline per entangling layer (from #105 layers or interaction segments)

1. **Reuse analysis** (Sec. III-A): atoms already in entanglement-zone pair sites for the gate stay put.
2. **Placement choice**:
   - **Agnostic**: minimize Σ Euclidean distance of required transfers into entanglement pairs (ZAC-style).
   - **Aware**: A* / best-first over partial assignments; node cost = Eq. (1) routing cost of the implied transition from previous placement (Sec. IV-A/B). Heuristic: admissible-enough lower bound using unpaired √ distances (document as inspired by Eqs. (3)–(5); full inadmissible RAP heuristic can land in follow-up if needed for #111 numbers).
3. **Emit** SLM↔AOD transfers + Move groups + Entangle2 in entanglement zone only.
4. **Validate** zone occupancy ≤ capacity; reject entangle outside entanglement zone; optional readout-zone check when `require_readout_zone`.

### Eq. (1) cost helper (Flux-friendly)

`movement_group_cost_sqrt(d_max_um, acceleration_m_s2) -> f64` and `sum_group_costs`.

### Out of scope for this PR

- Full #111 Table I numeric lock
- Full Sec. V-A BST AOD encoding (approximate compatibility via row/column non-crossing checks)
- Compaction (#108), resource Markdown (#110)

## Tests

- Entangle outside entanglement zone → error
- Occupancy overflow → error  
- Agnostic vs aware: aware cost ≤ agnostic on a fixture where reuse/routing matters
- Reuse skips no-op transfers
- Citations present in rustdoc

## Verdict path

Plan → adversarial review → implement → validate → submit stacked on #105.
