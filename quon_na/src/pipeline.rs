//! Neutral-atom schedule pipeline (post–`quantum.dynamic`).
//!
//! # Stages
//!
//! Callable without `quonc`. After monadic lowering + dynamic passes:
//!
//! 1. **Extract** interaction graph from the MLIR module (`mlir` feature)
//! 2. **`schedule_from_graph`** → **`schedule_entangling_layers`**
//! 3. **Zoned** (`schedule_zoned`) **or** flat AOD (`place` + `plan_aod_movement`)
//! 4. **Optional compaction** ([`compact_schedule`], best-effort)
//! 5. **Resource report** ([`build_resource_report`])
//!
//! Fixed (OpenQASM) physical passes live in `mlir_bridge::pipeline`.

use backend::{AodSpeedModelKind, NeutralAtomTarget, ZoneKind as BackendZoneKind};
use thiserror::Error;

use crate::compaction::{
    CompactionOptions, LegalityLimits, compact_schedule, infer_atom_dependencies,
};
use crate::entangling_schedule::schedule_entangling_layers;
use crate::graph::InteractionGraph;
use crate::movement::{MovementParams, plan_aod_movement};
use crate::placement::{PlacementStrategy, place};
use crate::report::{ResourceReport, build_resource_report};
use crate::schedule::ScheduleLayer;
use crate::schedule_entry::{GraphScheduleRequest, schedule_from_graph};
use crate::zoned::{PlacerMode, ZoneKind, ZoneSpec, ZonedArchitecture, schedule_zoned};

#[cfg(feature = "mlir")]
use melior::ir::Module;

#[cfg(feature = "mlir")]
use crate::extract::{ExtractError, extract_interaction_graph};

/// Which movement/placement backend to run after entangling-layer scheduling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NaBackendKind {
    /// Zoned RAP / ZAC path (#107) — default for multi-zone targets.
    #[default]
    Zoned,
    /// Flat AOD pair-bank planner (#106).
    FlatAod,
}

/// Options for the NA schedule pipeline.
#[derive(Clone, Copy, Debug)]
pub struct NaScheduleOptions {
    pub backend: NaBackendKind,
    pub placer: PlacerMode,
    pub compact: bool,
    pub placement: PlacementStrategy,
    /// When true, print stage summaries to stderr (matches `quonc --dump-ir`).
    pub dump_ir: bool,
}

impl Default for NaScheduleOptions {
    fn default() -> Self {
        Self {
            backend: NaBackendKind::Zoned,
            placer: PlacerMode::RoutingAgnostic,
            compact: true,
            placement: PlacementStrategy::RowMajor,
            dump_ir: false,
        }
    }
}

/// Artifacts from a successful NA schedule run.
#[derive(Clone, Debug)]
pub struct NaScheduleArtifacts {
    pub layers: Vec<ScheduleLayer>,
    pub resource_report: ResourceReport,
    pub logical_qubits: u64,
    pub request: GraphScheduleRequest,
}

/// Errors from the NA schedule pipeline.
#[derive(Debug, Error)]
pub enum NaPipelineError {
    #[error("{0}")]
    InvalidTarget(String),
    #[cfg(feature = "mlir")]
    #[error("interaction-graph extraction failed: {0}")]
    Extract(#[from] ExtractError),
    #[error("schedule_from_graph failed: {0}")]
    ScheduleFromGraph(#[from] crate::schedule_entry::ScheduleFromGraphError),
    #[error("entangling-layer scheduling failed: {0}")]
    Entangling(#[from] crate::entangling_schedule::EntanglingScheduleError),
    #[error("zoned scheduling failed: {0}")]
    Zoned(#[from] crate::zoned::ZonedScheduleError),
    #[error("placement failed: {0}")]
    Placement(#[from] crate::placement::PlacementError),
    #[error("AOD movement planning failed: {0}")]
    Movement(#[from] crate::movement::MovementPlanError),
    #[error("resource report failed: {0}")]
    Report(#[from] crate::report::ReportError),
}

/// Convert a loaded neutral-atom target into the zoned architecture RAP expects.
pub fn zoned_architecture(na: &NeutralAtomTarget) -> ZonedArchitecture {
    ZonedArchitecture {
        zones: na
            .zones
            .iter()
            .map(|z| ZoneSpec {
                zone_id: z.zone_id,
                kind: map_zone_kind(z.kind),
                rows: z.rows,
                cols: z.cols,
                origin_um: z.origin_um,
                site_pitch_um: z.site_pitch_um,
                pair_gap_um: z.pair_gap_um,
            })
            .collect(),
        acceleration_m_s2: na.movement.speed_model.acceleration_m_s2,
        trap_transfer_us: na.movement.trap_transfer_us.round() as u64,
        require_readout_zone: false,
        rydberg_range_um: na.interaction.rydberg_range_um,
        min_rydberg_spacing_um: na.interaction.min_rydberg_spacing_um,
        aod_min_separation_um: na.movement.min_row_col_separation_um,
    }
}

/// Flat AOD movement parameters derived from the same target JSON.
pub fn movement_params(na: &NeutralAtomTarget) -> MovementParams {
    let pair_gap_um = na
        .zones
        .iter()
        .find(|z| z.kind == BackendZoneKind::Entanglement)
        .and_then(|z| z.pair_gap_um)
        .unwrap_or(2.0);
    MovementParams {
        acceleration_m_s2: na.movement.speed_model.acceleration_m_s2,
        trap_transfer_us: na.movement.trap_transfer_us.round() as u64,
        rydberg_range_um: na.interaction.rydberg_range_um,
        min_rydberg_spacing_um: na.interaction.min_rydberg_spacing_um,
        min_row_col_separation_um: na.movement.min_row_col_separation_um,
        aod_rows: na.movement.aod_rows,
        aod_cols: na.movement.aod_cols,
        num_aods: na.movement.num_aods,
        pair_gap_um,
        pair_pitch_um: na.interaction.min_rydberg_spacing_um,
        return_home: false,
    }
}

/// Compaction options with optional geometry legality from the target.
pub fn compaction_options(na: &NeutralAtomTarget, greedy: bool) -> CompactionOptions {
    CompactionOptions {
        arch: Some(zoned_architecture(na)),
        legality: Some(LegalityLimits {
            rydberg_range_um: na.interaction.rydberg_range_um,
            min_rydberg_spacing_um: na.interaction.min_rydberg_spacing_um,
            aod_min_separation_um: na.movement.min_row_col_separation_um,
        }),
        greedy,
    }
}

/// Sanity-check that the target's speed model is the one `quon_na` implements.
pub fn validate_speed_model(na: &NeutralAtomTarget) -> Result<(), String> {
    match na.movement.speed_model.kind {
        AodSpeedModelKind::Sqrt => Ok(()),
    }
}

fn map_zone_kind(kind: BackendZoneKind) -> ZoneKind {
    match kind {
        BackendZoneKind::Storage => ZoneKind::Storage,
        BackendZoneKind::Entanglement => ZoneKind::Entanglement,
        BackendZoneKind::Readout => ZoneKind::Readout,
    }
}

/// Schedule from a post-dynamic MLIR module.
#[cfg(feature = "mlir")]
pub fn run_from_module<'c>(
    module: &Module<'c>,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
) -> Result<NaScheduleArtifacts, NaPipelineError> {
    validate_speed_model(na).map_err(NaPipelineError::InvalidTarget)?;
    let graph = extract_interaction_graph(module)?;
    let logical_qubits = graph.vertices.len() as u64;
    if opts.dump_ir {
        eprintln!(
            "--- interaction graph ---\nvertices={} interactions={}",
            graph.vertices.len(),
            graph.interactions.len()
        );
    }
    run_from_graph(graph, na, opts, Some(logical_qubits))
}

/// Schedule from a raw interaction graph (debug / stress entry).
pub fn run_from_graph(
    graph: InteractionGraph,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
    logical_qubits_override: Option<u64>,
) -> Result<NaScheduleArtifacts, NaPipelineError> {
    validate_speed_model(na).map_err(NaPipelineError::InvalidTarget)?;
    let logical_qubits = logical_qubits_override.unwrap_or(graph.vertices.len() as u64);

    let req = schedule_from_graph(graph)?;
    let max_pairs = na.interaction.max_parallel_entangling_pairs;
    let scheduled = schedule_entangling_layers(req, max_pairs)?;
    let mut req = scheduled.request;

    req = match opts.backend {
        NaBackendKind::Zoned => {
            let arch = zoned_architecture(na);
            let zoned = schedule_zoned(req, &arch, opts.placer)?;
            if opts.dump_ir {
                eprintln!(
                    "--- after zoned schedule ---\nlayers={} routing_cost={:.4} rearrangements={} transfers={}",
                    zoned.request.layers.len(),
                    zoned.routing_cost,
                    zoned.rearrangement_steps,
                    zoned.trap_transfers
                );
            }
            zoned.request
        }
        NaBackendKind::FlatAod => {
            let placed = place(req, opts.placement)?;
            let params = movement_params(na);
            let moved = plan_aod_movement(placed.request, &params)?;
            if opts.dump_ir {
                eprintln!(
                    "--- after flat AOD movement ---\nlayers={}",
                    moved.request.layers.len()
                );
            }
            moved.request
        }
    };

    if opts.compact && !req.layers.is_empty() {
        let deps = infer_atom_dependencies(&req.layers);
        let compact_opts = compaction_options(na, true);
        match compact_schedule(req.clone(), &deps, &compact_opts) {
            Ok(compacted) => {
                if opts.dump_ir {
                    eprintln!(
                        "--- after compaction ---\nlayers={} (was {})",
                        compacted.request.layers.len(),
                        req.layers.len()
                    );
                }
                req = compacted.request;
            }
            Err(e) => {
                // Compaction is best-effort: keep the pre-compact schedule.
                if opts.dump_ir {
                    eprintln!("--- compaction skipped ({e}) ---");
                }
            }
        }
    }

    let report = build_resource_report(&req.layers, None, Some(logical_qubits.max(1)))?;

    Ok(NaScheduleArtifacts {
        layers: req.layers.clone(),
        resource_report: report,
        logical_qubits,
        request: req,
    })
}
