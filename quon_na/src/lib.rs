//! MLIR-free neutral-atom backend domain types.
//!
//! This crate is intentionally additive: it defines serializable Rust data
//! structures for neutral-atom layouts, schedules, validation helpers, QEC
//! code-block expansion, and resource reports without registering dialects or
//! requiring an MLIR context.
//!
//! Interaction-graph extraction (#103) follows [Enola] (interaction graph /
//! dependency segments) and [Atomique] (layer-decayed `γ^l` edge weights); see
//! `docs/neutral_atom/architecture_model.md` §4.
//!
//! Placement (#104) maps logical qubits onto SLM sites with row-major,
//! degree-based, and interaction-clustering heuristics inspired by [Atomique]
//! Sec. III-B; see `docs/neutral_atom/architecture_model.md` §4.
//!
//! Entangling-layer scheduling (#105) uses Misra–Gries edge coloring on
//! commutation-group interaction graphs (Enola Sec. 3 / Theorem 1) and ASAP
//! layering on dependency-DAG segments; see `docs/neutral_atom/architecture_model.md` §4.
//!
//! Flat AOD movement (#106) uses a Quon interaction-pair bank with both-atom
//! duals and Enola-inspired per-axis conflict types / greedy longest-first IS
//! packing (not Enola one-atom duals; not RAP zoned routing). See
//! `docs/neutral_atom/architecture_model.md` §5–§6 and [`movement`].
//!
//! Zoned routing-aware placement (#107) follows [RAP] (placement cost = routing
//! cost, Eqs. (1)–(2)); see `docs/neutral_atom/architecture_model.md` §7.
//!
//! Schedule compaction (#108) is engineering glue: exclusive-cycle ASAP baseline
//! plus greedy E0 merge — **not** Enola-optimal true ASAP; see
//! [`compaction`] and `docs/neutral_atom/architecture_model.md` §4.
//!
//! Physical error-budget reporting uses optional target `error_model`
//! (ADR-0017 / `docs/adr/0017-na-physical-error-model.md`): analytic
//! `rate × schedule count` contributions on [`ResourceReport`], hard-fail when
//! `--emit-resource-report` requests a budget and the model is absent.

pub mod compaction;
#[cfg(feature = "mlir")]
pub mod dialect;
pub mod entangling_schedule;
#[cfg(feature = "mlir")]
pub mod extract;
pub mod geometry;
pub mod graph;
pub mod layout;
#[cfg(feature = "mlir")]
pub mod lower;
pub mod movement;
#[cfg(feature = "mlir")]
pub mod native_gate_decomp;
pub mod pipeline;
pub mod placement;
pub mod plan;
pub mod qec;
pub mod qec_schedule;
pub mod report;
pub mod schedule;
pub mod schedule_entry;
pub mod schedule_invariants;
pub mod schedule_view;
pub mod stats;
pub mod zoned;

pub use compaction::{
    CompactionError, CompactionOptions, CompactionResult, CriticalPathReport, LegalityLimits,
    ScheduleDependency, ScheduleDependencyKind, asap_schedule_layers, compact_schedule,
    feed_forward_dependencies, force_merge_layers, infer_atom_dependencies,
};
pub use entangling_schedule::{
    EntanglingScheduleError, EntanglingScheduleResult, LayerUtilization, capacity_layer_count,
    schedule_entangling_layers,
};
pub use graph::{
    DEFAULT_GAMMA, GraphError, Interaction, InteractionEdge, InteractionGraph, InteractionId,
    InteractionSegment, LogicalQubitId, SegmentKind, aggregate_edges, cubic_commutation_graph,
    erdos_renyi_commutation_graph, mark_critical_path, schedule_dependency_segment,
};
pub use layout::{
    AodTrapRef, AtomBinding, AtomId, AtomSite, NeutralAtomLayout, Position, SiteId, TrapBinding,
};
pub use movement::{
    BANK_ISOLATION_EPS_UM, CandidateLeg, InteractionPair, MoveSpec, MovementParams,
    MovementPlanError, MovementPlanResult, atom_moves_to_move_specs, check_entangling_geometry,
    ensure_interaction_pairs, legs_conflict, plan_aod_movement, try_transfer_into_occupied,
    verify_aod_legality, verify_entangling_geometry_predicates,
};
pub use placement::{
    PlacementError, PlacementResult, PlacementStrategy, SITE_PITCH_UM, grid_dims, place,
    placement_score,
};
pub use plan::{BackendStageInfo, plan_backend};
pub use qec::{
    CodeBlock, CodeBlockId, CodeFamily, NetRate, QecError, atoms_per_logical, ceil_div,
    code_blocks_from_expanded, repetition_n, surface_n,
};
pub use qec_schedule::{round_barrier_cuts, run_from_qec_workload};
pub use report::{
    BottleneckKind, ErrorBudgetContributions, RESOURCE_REPORT_EVIDENCE_DISCLAIMER,
    RESOURCE_REPORT_EVIDENCE_KIND, ReportError, ResourceReport, TemporalAtomMetrics,
    attach_qec_error_budget, build_resource_report, require_target_error_model,
    resource_report_to_json, resource_report_to_markdown, simultaneous_layer_time,
};
pub use schedule::{
    AtomMove, EntanglingAction, LocalGateKind, MeasurementBasis, MovementGroup, NeutralAtomAction,
    ReuseRegionId, ScheduleError, ScheduleLayer, TransferDirection, TrapTransfer,
};
pub use schedule_entry::{GraphScheduleRequest, ScheduleFromGraphError, schedule_from_graph};
pub use schedule_invariants::{cycle_is_monotonic, wait_barrier_ok};
pub use schedule_view::{
    NA_SCHEDULE_VIEW_KIND, NA_SCHEDULE_VIEW_VERSION, NaScheduleView, NaScheduleViewMeta,
    ScheduleViewZone,
};
pub use stats::{
    CompactionConfig, EffectiveConfig, NA_STATS_KIND, NA_STATS_SCHEMA_VERSION, NaStats,
    SearchDiagnostics, StageTimingsUs, VersionInfo, na_stats_to_json,
};
pub use zoned::{
    AWARE_NODE_BUDGET, AwareSearchOutcome, AwareSearchParams, PlacerMode, ZoneKind, ZoneSpec,
    ZonedArchitecture, ZonedScheduleError, ZonedScheduleResult, euclidean_um, movement_duration_us,
    routing_cost_eq1, schedule_zoned, schedule_zoned_with_aware_params, sqrt_d_max,
    toy_zoned_architecture, validate_zone_constraints,
};

#[cfg(feature = "mlir")]
pub use dialect::VerifyError;
#[cfg(feature = "mlir")]
pub use dialect::{
    ScheduleSpec, dump_schedule_text, verify, verify_mlir_text, verify_module, verify_schedule_spec,
};
#[cfg(feature = "mlir")]
pub use extract::{
    ExtractError, LocalGateExtract, extract_interaction_graph,
    extract_interaction_graph_and_local_gates,
    extract_interaction_graph_and_local_gates_with_gamma, extract_interaction_graph_with_gamma,
};
#[cfg(feature = "mlir")]
pub use lower::{ScheduleLowerError, ScheduleLowerParams, lower_layers, lower_schedule};
#[cfg(feature = "mlir")]
pub use pipeline::run_from_module;
pub use pipeline::{
    NaBackendKind, NaPipelineError, NaScheduleArtifacts, NaScheduleOptions, compaction_options,
    movement_params, run_from_graph, validate_speed_model, zoned_architecture,
};
