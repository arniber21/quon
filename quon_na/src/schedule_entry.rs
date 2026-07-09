//! Debug / stress-test entry point that accepts a raw interaction graph.
//!
//! [`schedule_from_graph`] bypasses Quon source and `quantum.dynamic` entirely.
//! It validates the graph and returns a [`GraphScheduleRequest`] with empty
//! schedule layers and no layout — a stable extension point for placement
//! ([`crate::placement::place`], #104), Misra–Gries entangling-layer scheduling
//! ([`crate::entangling_schedule::schedule_entangling_layers`], #105), AOD
//! movement (#106), zoned RAP (#107), and schedule compaction
//! ([`crate::compaction::compact_schedule`], #108) as a post-pass.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::{GraphError, InteractionGraph};
use crate::layout::NeutralAtomLayout;
use crate::schedule::ScheduleLayer;

/// Validated interaction graph plus the schedule slots later slices fill.
///
/// Distinct from the full Quon → `quantum.dynamic` pipeline. After
/// [`schedule_from_graph`], `layers` is empty and `layout` is `None` until
/// #104–#106 populate them. Call
/// [`crate::entangling_schedule::schedule_entangling_layers`] to fill `layers`
/// via Misra–Gries / ASAP (does not require `layout`). Optionally compact with
/// [`crate::compaction::compact_schedule`] (#108) after #105/#107.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphScheduleRequest {
    pub graph: InteractionGraph,
    /// Empty after [`schedule_from_graph`]; filled by
    /// [`crate::entangling_schedule::schedule_entangling_layers`] (#105);
    /// may be rewritten by [`crate::compaction::compact_schedule`] (#108).
    pub layers: Vec<ScheduleLayer>,
    /// Filled by [`crate::placement::place`] (#104); `None` after [`schedule_from_graph`].
    pub layout: Option<NeutralAtomLayout>,
}

/// Errors from [`schedule_from_graph`].
#[derive(Debug, Error, Clone, PartialEq)]
pub enum ScheduleFromGraphError {
    #[error(transparent)]
    InvalidGraph(#[from] GraphError),
}

/// Thin public entry: validate `graph`, wrap as a schedule request with empty
/// layers/layout.
///
/// Documented as a debug/stress-test path for property tests and
/// literature-comparable random graphs (e.g. 3-regular), not the production
/// Quon-program pipeline.
pub fn schedule_from_graph(
    graph: InteractionGraph,
) -> Result<GraphScheduleRequest, ScheduleFromGraphError> {
    graph.validate()?;
    Ok(GraphScheduleRequest {
        graph,
        layers: Vec::new(),
        layout: None,
    })
}
