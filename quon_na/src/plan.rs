//! Shared place → AOD-movement (or zoned routing) backend stage for both the
//! bare-qubit pipeline ([`crate::pipeline`]) and the hybrid QEC per-round
//! planner ([`crate::qec_schedule`]) — issue #317.
//!
//! The backend `match opts.backend` block (place + `plan_aod_movement`, or
//! `schedule_zoned_with_aware_params`) was duplicated between
//! `pipeline::finish_pipeline` and `qec_schedule::schedule_cnot_phase` with
//! divergent diagnostics/timing capture. Both now call [`plan_backend`], and
//! the QEC path can populate stage stats the same way as `run_from_graph`
//! (unblocking #307).
//!
//! # What is **not** shared (ADR-0016 hybrid shell)
//!
//! The hybrid QEC round-loop orchestration stays in `qec_schedule`:
//! per-round expansion, Wait barriers, serial Z-then-X phase split,
//! shared layout across rounds, and compaction with barrier cuts. Only the
//! place/AOD/entangle *stages* are shared — the round loop is never collapsed
//! into one synthetic graph.

use std::time::Instant;

use crate::graph::VertexId;
use crate::movement::plan_aod_movement;
use crate::pipeline::{
    NaBackendKind, NaPipelineError, NaScheduleOptions, movement_params, zoned_architecture,
};
use crate::placement::{PlacementStrategy, place};
use crate::schedule_entry::GraphScheduleRequest;
use crate::stats::SearchDiagnostics;
use crate::zoned::{AgnosticPlacerMechanism, PlacerMode, schedule_zoned_with_aware_params};
use backend::NeutralAtomTarget;

/// Wall-clock elapsed microseconds since `start` (saturating on overflow).
fn elapsed_us(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Timing and diagnostic info from the shared backend stage (place → AOD
/// movement or zoned routing). Used to populate [`crate::stats::NaStats`] on
/// both the bare-qubit and hybrid QEC paths (#307).
#[derive(Clone, Debug, Default)]
pub struct BackendStageInfo {
    /// `place` wall time (flat-AOD only). `None` for zoned or when placement
    /// was skipped (hybrid QEC shared layout — ADR-0016).
    pub placement_us: Option<u64>,
    /// `plan_aod_movement` wall time (flat-AOD only).
    pub movement_us: Option<u64>,
    /// `schedule_zoned_with_aware_params` wall time (zoned only).
    pub zoned_schedule_us: Option<u64>,
    pub placer_mode: Option<PlacerMode>,
    pub placement_strategy: Option<PlacementStrategy>,
    pub search_diagnostics: SearchDiagnostics,
    pub aware_search_status: Option<(u64, u64)>,
    /// Which routing-agnostic mechanism produced the zoned schedule (issue
    /// #300): `Some(Matching)`/`Some(GreedyFallback)` under
    /// [`PlacerMode::RoutingAgnostic`], `None` otherwise (flat-AOD or
    /// routing-aware). Mirrors `aware_search_status`'s "zoned-only" shape.
    pub agnostic_placer_mechanism: Option<AgnosticPlacerMechanism>,
}

/// Run the place → AOD movement (or zoned routing) backend on a
/// [`GraphScheduleRequest`] whose layers are already populated by
/// entangling-layer scheduling.
///
/// **Shared entry point** for the bare-qubit pipeline
/// ([`crate::pipeline::run_from_graph`] → `finish_pipeline`) and the hybrid
/// QEC per-round planner ([`crate::qec_schedule::schedule_cnot_phase`])
/// — issue #317. Both paths call this function for the place/AOD/zoned step;
/// the hybrid round-loop orchestration (per-round expansion, Wait barriers,
/// serial Z-then-X, shared layout) stays in `qec_schedule` (ADR-0016).
///
/// # Backend behavior
///
/// - **`FlatAod`**: places only when `req.layout` is `None` — the hybrid QEC
///   path shares a layout across CNOT phases (ADR-0016), so only the first
///   phase places — then plans AOD movement.
/// - **`Zoned`**: runs the zoned routing-aware scheduler (always; the zoned
///   scheduler handles layout internally).
///
/// Returns the updated request plus timing/diagnostic info for stats (#307).
pub fn plan_backend<V: VertexId>(
    mut req: GraphScheduleRequest<V>,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
) -> Result<(GraphScheduleRequest<V>, BackendStageInfo), NaPipelineError<V>> {
    let mut info = BackendStageInfo::default();

    req = match opts.backend {
        NaBackendKind::Zoned => {
            info.placer_mode = Some(opts.placer);
            let arch = zoned_architecture(na);
            let stage_started = Instant::now();
            let zoned =
                schedule_zoned_with_aware_params(req, &arch, opts.placer, opts.aware_search)?;
            info.zoned_schedule_us = Some(elapsed_us(stage_started));
            info.aware_search_status = Some((
                zoned.aware_search_completed_layers,
                zoned.aware_search_budget_exceeded_layers
                    + zoned.aware_search_no_legal_assignment_layers,
            ));
            let agnostic_placer_mechanism = if opts.placer == PlacerMode::RoutingAgnostic {
                // `GreedyFallback` if any layer used the greedy fallback
                // (very-large threshold or unrepaired conflict); `Matching`
                // otherwise — so a schedule reported as `Matching` is purely
                // matching-based, never a silent mix.
                Some(if zoned.agnostic_greedy_fallback_layers == 0 {
                    AgnosticPlacerMechanism::Matching
                } else {
                    AgnosticPlacerMechanism::GreedyFallback
                })
            } else {
                None
            };
            info.agnostic_placer_mechanism = agnostic_placer_mechanism;
            info.search_diagnostics = SearchDiagnostics {
                aware_search_completed_layers: Some(zoned.aware_search_completed_layers),
                aware_search_budget_exceeded_layers: Some(
                    zoned.aware_search_budget_exceeded_layers,
                ),
                aware_search_no_legal_assignment_layers: Some(
                    zoned.aware_search_no_legal_assignment_layers,
                ),
                aware_search_node_expansions: Some(zoned.aware_search_node_expansions),
                aware_search_node_budget: Some(opts.aware_search.node_budget as u64),
                aware_search_deepening_factor: Some(opts.aware_search.deepening_factor),
                aware_search_deepening_value: Some(opts.aware_search.deepening_value),
                aware_search_pruning_window: Some(opts.aware_search.pruning_window as u64),
                agnostic_placer_mechanism,
            };
            if opts.dump_ir {
                eprintln!(
                    "--- after zoned schedule ---\nlayers={} routing_cost={:.4} \
                     rearrangements={} transfers={} aware_search_completed={} \
                     aware_search_fell_back={}",
                    zoned.request.layers.len(),
                    zoned.routing_cost,
                    zoned.rearrangement_steps,
                    zoned.trap_transfers,
                    zoned.aware_search_completed_layers,
                    zoned.aware_search_budget_exceeded_layers
                        + zoned.aware_search_no_legal_assignment_layers,
                );
            }
            zoned.request
        }
        NaBackendKind::FlatAod => {
            info.placement_strategy = Some(opts.placement);
            if req.layout.is_none() {
                let stage_started = Instant::now();
                let placed = place(req, opts.placement)?;
                info.placement_us = Some(elapsed_us(stage_started));
                req = placed.request;
            }
            let params = movement_params(na);
            let stage_started = Instant::now();
            let moved = plan_aod_movement(req, &params)?;
            info.movement_us = Some(elapsed_us(stage_started));
            if opts.dump_ir {
                eprintln!(
                    "--- after flat AOD movement ---\nlayers={}",
                    moved.request.layers.len()
                );
            }
            moved.request
        }
    };

    Ok((req, info))
}

/// Saturating-sum two optional timings: `None + None = None`, else `Some(a+b)`.
fn sum_opt_us(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (None, None) => None,
        (None, Some(v)) | (Some(v), None) => Some(v),
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
    }
}

/// Accumulator for per-stage timing across all CNOT phases of a QEC workload.
/// Populated by [`plan_backend`] calls inside `schedule_cnot_phase` and used
/// by `schedule_expanded` to build [`crate::stats::NaStats`] (#307).
#[derive(Clone, Debug, Default)]
pub struct QecStageAccumulator {
    pub schedule_from_graph_us: u64,
    pub entangling_layers_us: u64,
    pub placement_us: Option<u64>,
    pub movement_us: Option<u64>,
    pub zoned_schedule_us: Option<u64>,
    pub placer_mode: Option<PlacerMode>,
    pub placement_strategy: Option<PlacementStrategy>,
    pub search_diagnostics: SearchDiagnostics,
    pub aware_search_status: Option<(u64, u64)>,
    pub agnostic_placer_mechanism: Option<AgnosticPlacerMechanism>,
}

impl QecStageAccumulator {
    /// Accumulate one phase's entangle + backend timing.
    pub fn accumulate_phase(
        &mut self,
        schedule_from_graph_us: u64,
        entangling_layers_us: u64,
        backend: &BackendStageInfo,
    ) {
        self.schedule_from_graph_us = self
            .schedule_from_graph_us
            .saturating_add(schedule_from_graph_us);
        self.entangling_layers_us = self
            .entangling_layers_us
            .saturating_add(entangling_layers_us);
        self.placement_us = sum_opt_us(self.placement_us, backend.placement_us);
        self.movement_us = sum_opt_us(self.movement_us, backend.movement_us);
        self.zoned_schedule_us = sum_opt_us(self.zoned_schedule_us, backend.zoned_schedule_us);
        // placer_mode / placement_strategy are the same across phases (they
        // come from opts); set if not already set.
        if self.placer_mode.is_none() {
            self.placer_mode = backend.placer_mode;
        }
        if self.placement_strategy.is_none() {
            self.placement_strategy = backend.placement_strategy;
        }
        // search_diagnostics: accumulate sums.
        let acc = &mut self.search_diagnostics;
        let inc = &backend.search_diagnostics;
        acc.aware_search_completed_layers = match (
            acc.aware_search_completed_layers,
            inc.aware_search_completed_layers,
        ) {
            (None, None) => None,
            (None, Some(v)) | (Some(v), None) => Some(v),
            (Some(a), Some(b)) => Some(a + b),
        };
        acc.aware_search_budget_exceeded_layers = match (
            acc.aware_search_budget_exceeded_layers,
            inc.aware_search_budget_exceeded_layers,
        ) {
            (None, None) => None,
            (None, Some(v)) | (Some(v), None) => Some(v),
            (Some(a), Some(b)) => Some(a + b),
        };
        acc.aware_search_no_legal_assignment_layers = match (
            acc.aware_search_no_legal_assignment_layers,
            inc.aware_search_no_legal_assignment_layers,
        ) {
            (None, None) => None,
            (None, Some(v)) | (Some(v), None) => Some(v),
            (Some(a), Some(b)) => Some(a + b),
        };
        acc.aware_search_node_expansions = match (
            acc.aware_search_node_expansions,
            inc.aware_search_node_expansions,
        ) {
            (None, None) => None,
            (None, Some(v)) | (Some(v), None) => Some(v),
            (Some(a), Some(b)) => Some(a + b),
        };
        // Node budget / deepening / pruning window are config echoes, not
        // cumulative counters — keep the first non-None.
        if acc.aware_search_node_budget.is_none() {
            acc.aware_search_node_budget = inc.aware_search_node_budget;
        }
        if acc.aware_search_deepening_factor.is_none() {
            acc.aware_search_deepening_factor = inc.aware_search_deepening_factor;
        }
        if acc.aware_search_deepening_value.is_none() {
            acc.aware_search_deepening_value = inc.aware_search_deepening_value;
        }
        if acc.aware_search_pruning_window.is_none() {
            acc.aware_search_pruning_window = inc.aware_search_pruning_window;
        }
        // aware_search_status: accumulate component-wise.
        match (self.aware_search_status, backend.aware_search_status) {
            (None, None) => {}
            (None, Some(v)) | (Some(v), None) => self.aware_search_status = Some(v),
            (Some((a1, b1)), Some((a2, b2))) => {
                self.aware_search_status = Some((a1 + a2, b1 + b2));
            }
        }
        // agnostic_placer_mechanism: config echo (same across phases), keep
        // the first non-None — but if any phase fell back to greedy, downgrade
        // a `Matching` to `GreedyFallback` so the aggregate is truthful.
        match (
            self.agnostic_placer_mechanism,
            backend.agnostic_placer_mechanism,
        ) {
            (None, v) => self.agnostic_placer_mechanism = v,
            (Some(AgnosticPlacerMechanism::GreedyFallback), _) => {}
            (_, Some(AgnosticPlacerMechanism::GreedyFallback)) => {
                self.agnostic_placer_mechanism = Some(AgnosticPlacerMechanism::GreedyFallback);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum_opt_us_none_none() {
        assert_eq!(sum_opt_us(None, None), None);
    }

    #[test]
    fn sum_opt_us_some_none() {
        assert_eq!(sum_opt_us(Some(5), None), Some(5));
        assert_eq!(sum_opt_us(None, Some(7)), Some(7));
    }

    #[test]
    fn sum_opt_us_some_some() {
        assert_eq!(sum_opt_us(Some(3), Some(4)), Some(7));
    }

    #[test]
    fn accumulator_accumulates() {
        let mut acc = QecStageAccumulator::default();
        let backend = BackendStageInfo {
            placement_us: Some(10),
            movement_us: Some(20),
            ..Default::default()
        };
        acc.accumulate_phase(5, 8, &backend);
        assert_eq!(acc.schedule_from_graph_us, 5);
        assert_eq!(acc.entangling_layers_us, 8);
        assert_eq!(acc.placement_us, Some(10));
        assert_eq!(acc.movement_us, Some(20));

        let backend2 = BackendStageInfo {
            movement_us: Some(30),
            ..Default::default()
        };
        acc.accumulate_phase(3, 2, &backend2);
        assert_eq!(acc.schedule_from_graph_us, 8);
        assert_eq!(acc.entangling_layers_us, 10);
        assert_eq!(acc.placement_us, Some(10)); // not re-placed
        assert_eq!(acc.movement_us, Some(50)); // 20 + 30
    }
}
