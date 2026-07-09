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
//! Zoned routing-aware placement (#107) follows [RAP] (placement cost = routing
//! cost, Eqs. (1)–(2)); see `docs/neutral_atom/architecture_model.md` §7.

#[cfg(feature = "mlir")]
pub mod dialect;
pub mod entangling_schedule;
#[cfg(feature = "mlir")]
pub mod extract;
pub mod graph;
pub mod layout;
pub mod placement;
pub mod qec;
pub mod report;
pub mod schedule;
pub mod schedule_entry;
pub mod zoned;

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
pub use placement::{
    PlacementError, PlacementResult, PlacementStrategy, SITE_PITCH_UM, grid_dims, place,
    placement_score,
};
pub use qec::{
    CodeBlock, CodeBlockId, CodeFamily, LogicalOp, NetRate, QecError, atoms_per_logical, ceil_div,
    expand_code_block, repetition_n, surface_n,
};
pub use report::{
    BottleneckKind, ReportError, ResourceReport, build_resource_report, resource_report_to_json,
    resource_report_to_markdown, simultaneous_layer_time,
};
pub use schedule::{
    AtomMove, EntanglingAction, MeasurementBasis, MovementGroup, NeutralAtomAction, ScheduleError,
    ScheduleLayer, TransferDirection, TrapTransfer,
};
pub use schedule_entry::{GraphScheduleRequest, ScheduleFromGraphError, schedule_from_graph};
pub use zoned::{
    PlacerMode, ZoneKind, ZoneSpec, ZonedArchitecture, ZonedScheduleError, ZonedScheduleResult,
    euclidean_um, movement_duration_us, routing_cost_eq1, schedule_zoned, sqrt_d_max,
    toy_zoned_architecture, validate_zone_constraints,
};

#[cfg(feature = "mlir")]
pub use extract::{ExtractError, extract_interaction_graph, extract_interaction_graph_with_gamma};
