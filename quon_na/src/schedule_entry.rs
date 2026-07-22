//! Debug / stress-test entry point that accepts a raw interaction graph.
//!
//! [`schedule_from_graph`] bypasses Quon source and `quantum.dynamic` entirely.
//! It validates the graph and returns a [`GraphScheduleRequest`] with empty
//! schedule layers and no layout — a stable extension point for placement
//! ([`crate::placement::place`], #104), Misra–Gries entangling-layer scheduling
//! ([`crate::entangling_schedule::schedule_entangling_layers`], #105), Quon flat
//! AOD movement with interaction-pair bank duals
//! ([`crate::movement::plan_aod_movement`], #106; Enola-inspired conflicts only),
//! zoned RAP (#107), and schedule compaction
//! ([`crate::compaction::compact_schedule`], #108) as a post-pass.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::{GraphError, InteractionGraph, LogicalQubitId, VertexId};
use crate::layout::NeutralAtomLayout;
use crate::schedule::ScheduleLayer;

/// Validated interaction graph plus the schedule slots later slices fill.
///
/// Generic over the vertex label `V` (default [`LogicalQubitId`]); see
/// [`InteractionGraph`]. The bare Quon-program path uses the default; the
/// hybrid QEC path (#318) uses `GraphScheduleRequest<AtomVertexId>`.
///
/// Distinct from the full Quon → `quantum.dynamic` pipeline. After
/// [`schedule_from_graph`], `layers` is empty and `layout` is `None` until
/// #104–#106 populate them. Call
/// [`crate::entangling_schedule::schedule_entangling_layers`] to fill `layers`
/// via Misra–Gries / ASAP (does not require `layout`), then
/// [`crate::movement::plan_aod_movement`] (#106) to expand with Transfer/Move
/// cycles (requires layout + interaction-pair bank). Optionally compact with
/// [`crate::compaction::compact_schedule`] (#108) after #105/#106/#107.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphScheduleRequest<V = LogicalQubitId> {
    pub graph: InteractionGraph<V>,
    /// Empty after [`schedule_from_graph`]; filled by
    /// [`crate::entangling_schedule::schedule_entangling_layers`] (#105);
    /// rewritten by [`crate::movement::plan_aod_movement`] (#106);
    /// may be rewritten by [`crate::compaction::compact_schedule`] (#108).
    pub layers: Vec<ScheduleLayer>,
    /// Filled by [`crate::placement::place`] (#104); enlarged with an
    /// interaction-pair bank by [`crate::movement::ensure_interaction_pairs`]
    /// (#106). `None` after [`schedule_from_graph`].
    pub layout: Option<NeutralAtomLayout>,
}

/// Errors from [`schedule_from_graph`].
///
/// Generic over the vertex label `V` (default [`LogicalQubitId`]); the graph
/// sub-error carries `V`.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum ScheduleFromGraphError<V = LogicalQubitId> {
    #[error(transparent)]
    InvalidGraph(#[from] GraphError<V>),
}

/// Thin public entry: validate `graph`, wrap as a schedule request with empty
/// layers/layout.
///
/// Documented as a debug/stress-test path for property tests and
/// literature-comparable random graphs (e.g. 3-regular), not the production
/// Quon-program pipeline.
pub fn schedule_from_graph<V: VertexId>(
    graph: InteractionGraph<V>,
) -> Result<GraphScheduleRequest<V>, ScheduleFromGraphError<V>> {
    graph.validate()?;
    Ok(GraphScheduleRequest {
        graph,
        layers: Vec::new(),
        layout: None,
    })
}
