//! Types for schedule compaction: dependencies, results, errors, options.
//!
//! # Invariants
//!
//! - [`ScheduleDependency`] is a hard ordering constraint that compaction must
//!   never violate.
//! - [`CompactionOptions::legality`] gates R2/R3 checks; without it geometry
//!   is unchecked even when a layout is present (B3 contract (b)).
//! - [`CompactionResult`] carries both ASAP and compacted makespans plus
//!   critical-path metadata.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::InteractionId;
use crate::layout::{AodTrapRef, AtomId};
use crate::schedule_entry::GraphScheduleRequest;
use crate::zoned::ZonedArchitecture;

/// Hard ordering constraints that compaction must never violate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDependency {
    /// Predecessor layer index in the input schedule (pre-compaction).
    pub before: u32,
    /// Successor layer index that may not move earlier than `before` completes.
    pub after: u32,
    pub kind: ScheduleDependencyKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleDependencyKind {
    /// Same atom appears in both layers (data dependence).
    AtomHazard,
    /// Explicit barrier / segment boundary.
    Barrier,
    /// Mid-circuit measurement must complete before dependent correction.
    Measurement,
    /// Classical feed-forward: measure → conditional correction (caller-supplied).
    FeedForward,
}

/// Critical-path metadata over the post-compaction layer DAG.
///
/// `critical_path_length` is the Enola Sec. 3 **lower-bound** style longest-path
/// length on dependency edges (a DAG property). It is independent of whether
/// exclusive-cycle ASAP serialized independent work.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriticalPathReport {
    /// Number of cycles = max(cycle)+1 after renumber / compaction.
    pub makespan_cycles: u32,
    /// Longest dependency-chain length in **post-merge cycle vertices** (N3).
    pub critical_path_length: u32,
    /// Stable **pre-merge input layer indices** that lie on some longest path
    /// (layers that were merged share the successor's cycle but keep ids listed).
    pub critical_layer_indices: Vec<u32>,
    /// Best-effort interaction ids when layers map cleanly to entangles.
    pub critical_interaction_ids: Vec<InteractionId>,
}

/// Result of exclusive-cycle ASAP and optional greedy compaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactionResult {
    pub request: GraphScheduleRequest,
    pub asap_makespan_cycles: u32,
    pub compacted_makespan_cycles: u32,
    pub critical_path: CriticalPathReport,
    /// True if greedy pass merged at least one pair of layers.
    pub compacted: bool,
}

/// Errors from schedule compaction (#108).
#[derive(Debug, Error, Clone, PartialEq)]
pub enum CompactionError {
    #[error("empty schedule")]
    EmptySchedule,
    #[error("schedule layer conflict after compaction: {0}")]
    Conflict(String),
    #[error("occupancy conflict after compaction: {0}")]
    Occupancy(String),
    #[error("zone constraint violated after compaction: {0}")]
    Zone(String),
    #[error("physical legality violated after compaction: {0}")]
    PhysicalLegality(String),
    #[error("merge class forbidden in v0: {0}")]
    ForbiddenMergeClass(String),
    #[error("feed-forward / measurement dependency would be violated")]
    DependencyViolation,
    #[error("invalid dependency edge {0:?} → {1:?}")]
    InvalidDependency(u32, u32),
    #[error("layout required for position-aware legality")]
    LayoutRequired,
}

/// Geometry / AOD limits for post-merge physical legality.
#[derive(Clone, Debug, PartialEq)]
pub struct LegalityLimits {
    pub rydberg_range_um: f64,
    pub min_rydberg_spacing_um: f64,
    pub aod_min_separation_um: f64,
}

/// Options for [`super::compact_schedule`].
#[derive(Clone, Debug, Default)]
pub struct CompactionOptions {
    /// When set with layout, run zone checks after compaction (best-effort; see B3).
    pub arch: Option<ZonedArchitecture>,
    /// Position-aware R2/R3 runs **iff** this is `Some` **and** `request.layout`
    /// is `Some` (B3 contract **(b)**). `None` ⇒ geometry unchecked (AC2 may
    /// accept physically illegal E0 merges). Callers who need geometry must set
    /// `legality`; layout alone does **not** enable R2/R3.
    pub legality: Option<LegalityLimits>,
    /// If true, run greedy compaction after ASAP; if false, ASAP-only baseline.
    pub greedy: bool,
}

/// Placeholder AOD ref emitted by #107 `schedule_zoned` (all zeros).
pub(crate) const PLACEHOLDER_AOD: AodTrapRef = AodTrapRef {
    aod_id: 0,
    row: 0,
    col: 0,
};

/// Re-export AtomId for merge module.
pub(crate) use AtomId as _AtomIdReexport;
