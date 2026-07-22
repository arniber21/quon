# Shared NA place/AOD/entangle backend stage

## Context

The bare-qubit NA pipeline (`pipeline::run_from_graph` → `finish_pipeline`) and
the hybrid QEC pipeline (`qec_schedule::run_from_qec_workload` →
`schedule_expanded` → `schedule_round` → `schedule_cnot_phase`) both plan a
`GraphScheduleRequest` through the same backend stages:

1. **Entangle**: `schedule_from_graph` → `schedule_entangling_layers` (Misra–Gries
   / ASAP).
2. **Place/AOD or zoned**: `place` + `plan_aod_movement` (flat-AOD) or
   `schedule_zoned_with_aware_params` (zoned).

These stages were duplicated: `finish_pipeline` (bare) and `schedule_cnot_phase`
(hybrid) each had their own `match opts.backend { … }` block with divergent
diagnostics capture, timing instrumentation, and error mapping. The hybrid path
also left `NaScheduleArtifacts.stats: None`, blocking `--emit-na-stats` (#307).

The **hybrid round-loop shell** — per-round expansion, Wait barriers, serial
Z-then-X phase split, shared layout across rounds, compaction with barrier cuts
— is distinct from the per-phase planning and must stay (ADR-0016). The
deletion test: the duplicated place/move/entangle wiring is shallow; the round
loop + Wait barriers + Z-then-X split earns keep.

## Decision

Extract the backend `match opts.backend` block (place + `plan_aod_movement`, or
`schedule_zoned_with_aware_params`) into a single shared function
`plan_backend` in a new `quon_na::plan` module. Both `pipeline::finish_pipeline`
and `qec_schedule::schedule_cnot_phase` call `plan_backend` for the place/AOD
(or zoned) step. The entangle step (`schedule_from_graph` +
`schedule_entangling_layers`) is already shared through individual function
calls; both paths call these same functions.

`plan_backend` is generic over `V: VertexId` (ADR-0029), so the bare path calls
it with `GraphScheduleRequest<LogicalQubitId>` and the hybrid path with
`GraphScheduleRequest<AtomVertexId>`. It returns a `BackendStageInfo` carrying
timing and zoned-search diagnostics, used to populate `NaStats` on both paths.

The hybrid path's shared-layout behavior (skip re-placement when
`req.layout.is_some()` — ADR-0016) is preserved inside `plan_backend`: the
first CNOT phase places; subsequent phases reuse the layout and only run AOD
movement. The bare path always has `layout: None` coming in, so it always
places — unchanged behavior.

`schedule_expanded` now accumulates per-phase `BackendStageInfo` across all
CNOT phases (via `QecStageAccumulator`) and builds `NaStats` with meaningful
stage timings, setting `stats: Some(...)` instead of `None`. This unblocks #307
(`--emit-na-stats`) for the hybrid QEC path.

### What is NOT shared (ADR-0016 hybrid shell)

The hybrid round-loop orchestration stays in `qec_schedule`:
- Per-round expansion via `quon_qec::expand_workload`.
- Wait barriers after each memory round.
- Serial Z-then-X phase split (`z_cnot_count` / mid H / X CXs / after H).
- Shared layout across rounds (first phase places, subsequent phases reuse).
- Compaction with barrier cuts (`round_barrier_cuts`) and lattice-surgery
  FeedForward dependencies.

The hybrid path is **never collapsed** into one synthetic graph through the
whole program — ADR-0016 invariants (round barriers, Z-then-X split, shared
layout across rounds) are preserved.

## Consequences

- Both paths share a single place/AOD/zoned entry point; a bug fix or
  diagnostic improvement in `plan_backend` applies to both pipelines.
- The QEC path now populates `NaStats` with per-stage timings, search
  diagnostics, and effective-config echo — `stats: Some(...)` instead of `None`
  (#307).
- No new schedule IR type is introduced; `ScheduleLayer → ScheduleSpec` remains
  the only converter (ADR-0011). `plan_backend` operates on
  `GraphScheduleRequest<V>` / `ScheduleLayer`, the existing planner-internal
  representation.
- `NaScheduleArtifacts::stats` is now `Some` from all three entry points
  (`run_from_graph`, `run_from_module`, `run_from_qec_workload`).
- Bare-qubit path observable artifacts (layers, resource report) are unchanged.

## References

- #317 — issue
- #307 — `--emit-na-stats` (unblocked by this change)
- #201 — `ScheduleLayer → ScheduleSpec` converter (ADR-0011 decision context)
- ADR-0016 — hybrid QEC round expansion (round barriers, Z-then-X, shared layout)
- ADR-0011 — `quantum.na` canonical schedule IR; `ScheduleLayer` planner-internal
- ADR-0007 — separate `quantum.na` MLIR dialect as NA schedule IR
- ADR-0029 — atom-indexed hybrid interaction graph (`V: VertexId` generic)
