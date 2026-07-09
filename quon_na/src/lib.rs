//! MLIR-free neutral-atom backend domain types.
//!
//! This crate is intentionally additive: it defines serializable Rust data
//! structures for neutral-atom layouts, schedules, validation helpers, and
//! resource reports without registering dialects or requiring an MLIR context.
//!
//! Interaction-graph extraction (#103) follows [Enola] (interaction graph /
//! dependency segments) and [Atomique] (layer-decayed `γ^l` edge weights); see
//! `docs/neutral_atom/architecture_model.md` §4.

#[cfg(feature = "mlir")]
pub mod dialect;
#[cfg(feature = "mlir")]
pub mod extract;
pub mod graph;
pub mod layout;
pub mod report;
pub mod schedule;
pub mod schedule_entry;

pub use graph::{
    DEFAULT_GAMMA, GraphError, Interaction, InteractionEdge, InteractionGraph, InteractionId,
    InteractionSegment, LogicalQubitId, SegmentKind, aggregate_edges, cubic_commutation_graph,
    erdos_renyi_commutation_graph, mark_critical_path, schedule_dependency_segment,
};
pub use layout::{
    AodTrapRef, AtomBinding, AtomId, AtomSite, NeutralAtomLayout, Position, SiteId, TrapBinding,
};
pub use report::ResourceReport;
pub use schedule::{
    AtomMove, EntanglingAction, MeasurementBasis, MovementGroup, NeutralAtomAction, ScheduleError,
    ScheduleLayer, TransferDirection, TrapTransfer,
};
pub use schedule_entry::{GraphScheduleRequest, ScheduleFromGraphError, schedule_from_graph};

#[cfg(feature = "mlir")]
pub use extract::{ExtractError, extract_interaction_graph, extract_interaction_graph_with_gamma};
